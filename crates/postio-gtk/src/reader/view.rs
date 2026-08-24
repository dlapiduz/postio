//! The hardened `WebView`: `postio-lu6`.
//!
//! A message body is hostile input that has to render correctly anyway. The
//! four rules, each backed by a real API rather than a promise:
//!
//! * **JavaScript is off**, at the `WebKitSettings` level, along with every
//!   other scripting-adjacent surface (`WebGL`, `WebRTC`, IndexedDB-style
//!   storage, the offline application cache) — a script disabled by policy in
//!   one place and reachable through another is not disabled.
//! * **Nothing is fetched.** [`sanitize::sanitize_body`] never leaves a
//!   remote `src` in the markup unless the caller explicitly allows it
//!   (`postio-xxz`), so there is nothing in the DOM to fetch in the first
//!   place; the `WebView` also gets its own ephemeral `NetworkSession`,
//!   isolated from anything else in the process and backed by no disk cache
//!   or cookie jar. Two independent reasons the tracking-pixel fixture
//!   requests nothing.
//! * **Inline images stay local.** `cid:` references resolve through
//!   [`scheme::register`] against whatever [`BlobSource`] the caller hands
//!   in — a blob-store read, never a network round trip.
//! * **A click never navigates the pane.** [`decide-policy`][decide] fires
//!   for every frame navigation, including our own `load_html`; only a
//!   navigation whose `NavigationType` is [`LinkClicked`] gets intercepted
//!   and handed to [`gtk::UriLauncher`] instead.
//!
//! [decide]: https://webkitgtk.org/reference/webkit2gtk/stable/signal.WebView.decide-policy.html
//! [`LinkClicked`]: webkit6::NavigationType::LinkClicked

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use gtk::glib;
use postio_model::message::MessageBody;
use webkit6::prelude::*;

use super::allowlist::RemoteImageAllowList;
use super::banner::RemoteImageBanner;
use super::scheme::{self, BlobSource};
use crate::resources;
use postio_body::quote;
use postio_body::sanitize::{self, RemoteImages};

/// The security origin every rendered message loads under.
///
/// A fixed, non-`http(s)` scheme so a message's content is never same-origin
/// with any real site — nothing it contains gets that site's cookies, and
/// nothing on that site sees this page as one of its own frames. Nothing is
/// ever registered to handle this scheme, so a relative reference a sender
/// left in place resolves to a fetch that fails closed rather than one that
/// quietly reaches a host.
pub const DOCUMENT_BASE_URI: &str = "postio-reader:///";

/// The message currently on screen, kept so the banner's two actions can ask
/// for a re-render without the caller doing it for them.
struct Open {
    body: MessageBody,
    sender: Option<String>,
}

/// Called with how many remote references the pane is currently holding
/// back, every time a render decides that count anew.
type RenderedHandler = Box<dyn Fn(u32)>;

/// The reading pane: a hardened `WebView`, and the remote-image banner
/// (`postio-xxz`) that sits above it.
///
/// `Clone` is cheap — every field is a GObject reference or an `Rc` — so a
/// caller can hand a `Reader` to more than one closure without fighting the
/// borrow checker over who owns it.
#[derive(Clone)]
pub struct Reader {
    container: gtk::Box,
    view: webkit6::WebView,
    banner: Rc<RemoteImageBanner>,
    allowlist: Rc<RefCell<RemoteImageAllowList>>,
    open: Rc<RefCell<Option<Open>>>,
    /// Which [`Absent`] the pane is explaining, when it has no body to draw.
    /// `None` whenever a body is on screen — the two are exclusive, and
    /// `render` and `clear` both say so.
    absent: Rc<std::cell::Cell<Option<Absent>>>,
    /// Terms to paint where they appear in the body. Empty for ordinary
    /// reading; set while a search is what put the message on screen.
    highlight: Rc<RefCell<Vec<String>>>,
    /// What came with the message, per canvas 1b — and the way into the
    /// parts panel. See [`crate::parts::Chips`].
    chips: crate::parts::Chips,
    /// Called every time a render settles how many remote references are
    /// being held back — initial render, and again if the banner's "show
    /// once" or "always allow" changes it. See [`connect_rendered`].
    ///
    /// [`connect_rendered`]: Reader::connect_rendered
    rendered: Rc<RefCell<Vec<RenderedHandler>>>,
    /// Called when `p` asks to see the parts panel for whatever is showing,
    /// with no chip to click — see [`Reader::connect_parts_requested`].
    on_parts_requested: Rc<RefCell<Vec<PartsRequestedHandler>>>,
}

/// What [`Reader::connect_parts_requested`] holds.
type PartsRequestedHandler = Box<dyn Fn()>;

/// Why the reading pane has no body to draw.
///
/// Issue #70, Cause A: all four of these used to render as a blank pane, so
/// a mailbox mid-backfill was indistinguishable from a broken application.
/// They are separate because the right response differs — three are worth
/// waiting for and one is finished, and only two are worth retrying.
///
/// This is the reader's half of the "partial" state every Postio surface
/// owes: headers synced, body not yet, which is the *ordinary* condition of
/// a mailbox that has just been added rather than a fault.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Absent {
    /// Headers are synced and the backfill has not reached the body yet.
    ///
    /// The overwhelmingly common one, and not an error: `request_body`
    /// queues a fetch and returns, so every message is in this state until
    /// the queue drains.
    Partial,
    /// Not downloaded, and nothing is downloading — the engine is offline.
    Offline,
    /// A body was recorded but its bytes are not in the blob store.
    ///
    /// Rare, and a real fault: the database and the blob directory disagree.
    Missing,
    /// Downloaded, and the message genuinely has no text or HTML part.
    Empty,
    /// `\Draft`, but written by another client: there is no local composer
    /// buffer behind the row, so there is nothing here to resume editing.
    ///
    /// Not a retryable wait like [`Absent::Partial`] or [`Absent::Offline`] --
    /// downloading its body would only ever produce something to read, never
    /// something to edit, so promising a retry would be a promise the reader
    /// cannot keep. See #175.
    ForeignDraft,
}

/// What the pane says for each [`Absent`], as the document's body.
///
/// A free function returning a string so the words and the key are testable
/// without a display — the assertions that matter here are "these four do
/// not say the same thing" and "the retryable ones name the retry key",
/// neither of which needs WebKit to have painted anything.
fn absent_html(state: Absent) -> String {
    // `R` is the registry's alternate binding for `Refresh`, and already the
    // canvas' retry key for the list's empty and error plates. One retry key
    // for the whole application, not one per surface.
    const RETRY: &str = "Press <kbd>R</kbd> to check for new mail now.";
    let (heading, detail) = match state {
        Absent::Partial => (
            "Downloading this message",
            "Its headers are here; the body has not arrived yet. It will              appear as soon as it does."
                .to_owned(),
        ),
        Absent::Offline => (
            "Not downloaded yet",
            format!(
                "Postio is offline, so this message's body is not on this                  machine. It will arrive when the connection returns. {RETRY}"
            ),
        ),
        Absent::Missing => (
            "This message's body is missing",
            format!(
                "The message is in the local store but its body is not, so                  there is nothing here to show. {RETRY}"
            ),
        ),
        Absent::Empty => (
            "This message has no body",
            "Nothing arrived with it but the headers — that is the whole              message, not a fault."
                .to_owned(),
        ),
        Absent::ForeignDraft => (
            "Written on another device",
            "This draft was started in another mail client. Postio can              show it here but cannot edit it."
                .to_owned(),
        ),
    };
    format!(
        "<div class=\"postio-absent\">\
         <p class=\"postio-absent-heading\">{heading}</p>\
         <p class=\"postio-absent-detail\">{detail}</p>\
         </div>"
    )
}

impl Reader {
    /// Build a reader that resolves inline (`cid:`) images through `source`.
    ///
    /// The remote-image allow list loads from `$XDG_STATE_HOME` here, once,
    /// and stays in memory for the reader's life — the same lifetime as the
    /// window it lives in, so there is never a second reader to disagree
    /// with it about who is allow-listed.
    pub fn new(source: Rc<dyn BlobSource>) -> Self {
        Self::with_allowlist(
            source,
            RemoteImageAllowList::load(),
            RemoteImageAllowList::path(),
        )
    }

    /// As [`new`](Self::new), with the allow list a caller already has and
    /// an explicit path to persist a change to — what the tests use to point
    /// it at a scratch file instead of the developer's own state directory.
    pub fn with_allowlist(
        source: Rc<dyn BlobSource>,
        allowlist: RemoteImageAllowList,
        allowlist_path: std::path::PathBuf,
    ) -> Self {
        let network_session = webkit6::NetworkSession::new_ephemeral();
        network_session.set_persistent_credential_storage_enabled(false);

        let context = webkit6::WebContext::new();
        scheme::register(&context, source);

        let view = webkit6::WebView::builder()
            .web_context(&context)
            .network_session(&network_session)
            .settings(&hardened_settings())
            .hexpand(true)
            .vexpand(true)
            .build();
        view.add_css_class("postio-reader-view");
        view.set_accessible_role(gtk::AccessibleRole::Article);
        view.connect_decide_policy(handle_decide_policy);

        let banner = Rc::new(RemoteImageBanner::new());

        let chips = crate::parts::Chips::new();

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&banner.widget());
        container.append(&view);
        container.append(&chips.widget());

        let reader = Reader {
            container,
            view,
            banner,
            allowlist: Rc::new(RefCell::new(allowlist)),
            open: Rc::new(RefCell::new(None)),
            absent: Rc::new(std::cell::Cell::new(None)),
            highlight: Rc::new(RefCell::new(Vec::new())),
            chips,
            rendered: Rc::new(RefCell::new(Vec::new())),
            on_parts_requested: Rc::new(RefCell::new(Vec::new())),
        };

        // The banner's buttons are children of `reader.banner`'s own widget
        // tree, so a closure their "clicked" signal owns must not hold a
        // *strong* `Rc<RemoteImageBanner>` back to it — that would be a
        // button owning (via the signal) a closure owning (via the Rc) the
        // struct that owns the button, a cycle nothing would ever free.
        // `view`, `open` and `allowlist` hold no reference back to the
        // banner, so they can be captured strongly with no such risk.
        let banner_weak = Rc::downgrade(&reader.banner);
        {
            let view = reader.view.clone();
            let open = Rc::clone(&reader.open);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let banner_weak = banner_weak.clone();
            reader.banner.connect_show_once(move || {
                if let Some(banner) = banner_weak.upgrade() {
                    render_open(
                        &view,
                        &banner,
                        &open,
                        &highlight,
                        RemoteImages::Allowed,
                        &rendered,
                    );
                }
            });
        }
        {
            let view = reader.view.clone();
            let open = Rc::clone(&reader.open);
            let allowlist = Rc::clone(&reader.allowlist);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            reader.banner.connect_always_allow(move || {
                let sender = open.borrow().as_ref().and_then(|o| o.sender.clone());
                if let Some(sender) = sender {
                    let mut allowlist = allowlist.borrow_mut();
                    allowlist.allow(&sender);
                    if let Err(error) = allowlist.save_to(&allowlist_path) {
                        glib::g_warning!(
                            "postio",
                            "could not save the remote-image allow list: {error}"
                        );
                    }
                }
                if let Some(banner) = banner_weak.upgrade() {
                    render_open(
                        &view,
                        &banner,
                        &open,
                        &highlight,
                        RemoteImages::Allowed,
                        &rendered,
                    );
                }
            });
        }

        reader.clear();
        reader
    }

    /// The widget to place in [`crate::shell::Shell::reader`]: the banner
    /// and the `WebView`, stacked.
    pub fn widget(&self) -> gtk::Widget {
        self.container.clone().upcast()
    }

    /// The underlying `WebView` — test-facing, e.g. to watch `load-changed`
    /// for whether a render has finished yet.
    pub fn view(&self) -> &webkit6::WebView {
        &self.view
    }

    /// Whether the remote-image banner is currently shown.
    pub fn banner_visible(&self) -> bool {
        self.banner.is_visible()
    }

    /// The banner's "always allow" button label, naming whichever sender it
    /// would exempt.
    pub fn banner_always_allow_label(&self) -> String {
        self.banner.always_allow_label()
    }

    /// Simulate clicking the banner's "always allow" — what a test uses in
    /// place of a synthesized pointer click.
    pub fn click_always_allow(&self) {
        self.banner.emit_always_allow();
    }

    /// As [`click_always_allow`](Self::click_always_allow), for "show once".
    pub fn click_show_once(&self) {
        self.banner.emit_show_once();
    }

    /// Render `body` into the pane.
    ///
    /// `sender` is the allow-list key: with a sender already on the standing
    /// allow list, remote images load without the banner appearing at all.
    /// Otherwise they stay blocked until the banner's "show once" or "always
    /// allow" is used — both re-render through this same [`Open`] state, so
    /// a caller never has to.
    pub fn render(&self, body: &MessageBody, sender: Option<&str>) {
        self.absent.set(None);
        *self.open.borrow_mut() = Some(Open {
            body: body.clone(),
            sender: sender.map(str::to_owned),
        });
        let allowed = sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(sender));
        let remote = if allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        render_open(
            &self.view,
            &self.banner,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
    }

    /// Called every time a render settles how many remote references are
    /// being held back — see [`RenderedHandler`].
    ///
    /// Fires on the initial render and again whenever the banner's "show
    /// once" or "always allow" changes the count, so a caller wiring the
    /// parts panel's [`crate::parts::PartsPanel::set_held_back`] never goes
    /// stale.
    pub fn connect_rendered(&self, handler: impl Fn(u32) + 'static) {
        self.rendered.borrow_mut().push(Box::new(handler));
    }

    /// Draw the message's attachments as chips under the body.
    ///
    /// Metadata only, and deliberately: a chip is drawn from what
    /// `BODYSTRUCTURE` already said, so a message nothing has been fetched
    /// for still shows what came with it. See [`crate::parts`].
    pub fn set_attachments(&self, root: &str, parts: &[postio_model::Attachment]) {
        self.chips.set_parts(root, parts);
    }

    /// Called when one of those chips is activated.
    ///
    /// The chip does not act — it asks. Whoever wires this opens
    /// [`crate::parts::PartsPanel`], which is where the verbs live.
    pub fn connect_attachment(&self, handler: impl Fn(&crate::parts::Node) + 'static) {
        self.chips.connect_activated(handler);
    }

    /// Ask for the parts panel — `p`, the keyboard's way in when there is no
    /// chip to click. Same destination as [`Reader::connect_attachment`],
    /// with no particular part in hand: it opens on whatever the pane is
    /// showing, same as clicking any chip does today.
    pub fn connect_parts_requested(&self, handler: impl Fn() + 'static) {
        self.on_parts_requested.borrow_mut().push(Box::new(handler));
    }

    /// Fires what [`Reader::connect_parts_requested`] is listening for.
    pub fn request_parts(&self) {
        for handler in self.on_parts_requested.borrow().iter() {
            handler();
        }
    }

    /// Paint `terms` wherever they appear in the body.
    ///
    /// What canvas 2b means by "preview · match highlighted": the same
    /// hardened pane, with the reason this message is a hit picked out in it.
    /// Marking happens after sanitizing (see [`crate::search::mark_html`]),
    /// so nothing here loosens what the reader will render. An empty list
    /// turns it off, which is the state ordinary reading is in.
    ///
    /// Takes effect on the next [`Reader::render`]; the caller sets the terms
    /// and then shows the message, which is the order a search does it in
    /// anyway.
    pub fn set_highlight(&self, terms: Vec<String>) {
        *self.highlight.borrow_mut() = terms;
    }

    /// Say why there is no body, instead of drawing nothing.
    ///
    /// The pane follows the cursor now (#70, Cause B), so this is reached on
    /// most cursor movements in a mailbox that has not finished backfilling.
    /// It stays inside the same hardened `WebView` rather than becoming an
    /// overlay: one widget owns the pane, and the document is built by the
    /// same [`wrap_document`] with remote images blocked, so a state plate
    /// can no more reach the network than a message can.
    pub fn show_absent(&self, state: Absent) {
        *self.open.borrow_mut() = None;
        self.absent.set(Some(state));
        self.banner.set_visible(false);
        self.view.load_html(
            &wrap_document(&absent_html(state), RemoteImages::Blocked),
            Some(DOCUMENT_BASE_URI),
        );
        // No body drawn, so nothing is being held back either — a caller
        // watching `connect_rendered` must not keep showing the previous
        // message's count.
        for handler in self.rendered.borrow().iter() {
            handler(0);
        }
    }

    /// Which [`Absent`] the pane is explaining, or `None` if it has a body.
    ///
    /// The seam the wiring tests assert on: proving the *application* stopped
    /// drawing blank panes means asking what the reader was told, which does
    /// not require driving WebKit to a paint.
    pub fn absent(&self) -> Option<Absent> {
        self.absent.get()
    }

    /// Empty the pane — nothing selected, or the selection closed.
    pub fn clear(&self) {
        *self.open.borrow_mut() = None;
        self.absent.set(None);
        self.banner.set_visible(false);
        self.view.load_html(
            &wrap_document("", RemoteImages::Blocked),
            Some(DOCUMENT_BASE_URI),
        );
        for handler in self.rendered.borrow().iter() {
            handler(0);
        }
    }
}

/// Re-render whatever is in `open` at `remote`'s policy, and put the banner
/// in step with the result.
///
/// A free function, not a method, so the two banner-signal closures can call
/// it through weak/`Rc` captures without holding a whole `Reader` (which
/// would capture `container` — and so the banner and the button doing the
/// capturing — in a reference cycle nothing would ever free).
fn render_open(
    view: &webkit6::WebView,
    banner: &RemoteImageBanner,
    open: &Rc<RefCell<Option<Open>>>,
    highlight: &Rc<RefCell<Vec<String>>>,
    remote: RemoteImages,
    rendered: &Rc<RefCell<Vec<RenderedHandler>>>,
) {
    let (body, sender) = {
        let guard = open.borrow();
        let Some(current) = guard.as_ref() else {
            return;
        };
        (current.body.clone(), current.sender.clone())
    };
    let (content, remote_blocked) = body_html(&body, remote);
    // After sanitizing and quote-folding, never before: ammonia would strip
    // the `<mark>` as an unknown tag, and there is no point running a matcher
    // over markup that has not been cleaned yet.
    let content = crate::search::mark_html(&content, &highlight.borrow());

    banner.set_sender(sender.as_deref());
    banner.set_visible(remote == RemoteImages::Blocked && remote_blocked > 0);

    let document = wrap_document(&content, remote);
    view.load_html(&document, Some(DOCUMENT_BASE_URI));

    for handler in rendered.borrow().iter() {
        handler(remote_blocked);
    }
}

/// The body markup: sanitized and quote-folded, but not yet wrapped in the
/// document template [`wrap_document`] adds. The count is how many remote
/// references were stripped — see [`sanitize::Sanitized::remote_blocked`].
fn body_html(body: &MessageBody, remote: RemoteImages) -> (String, u32) {
    if let Some(html) = body.html.as_deref().filter(|html| !html.trim().is_empty()) {
        let sanitized = sanitize::sanitize_body(html, remote);
        return (
            quote::fold_html_quotes(&sanitized.html),
            sanitized.remote_blocked,
        );
    }
    if let Some(text) = body.text.as_deref().filter(|text| !text.trim().is_empty()) {
        return (quote::text_to_html(text), 0);
    }
    (String::new(), 0)
}

/// Every scripting-adjacent `WebKitSettings` flag, turned off.
///
/// JavaScript is the headline, but each of these is a surface JavaScript
/// being off does not automatically close: WebGL and WebRTC run without a
/// `<script>` tag executing, and the storage APIs persist to disk regardless
/// of whether anything is currently running to read them back.
///
/// Three settings this build's WebKitGTK once had — offline application
/// cache, DNS prefetching, hyperlink auditing — are not here: each is
/// deprecated as of the WebKit version this crate targets because the engine
/// removed the underlying feature outright, so there is nothing left for the
/// setter to turn off.
fn hardened_settings() -> webkit6::Settings {
    let settings = webkit6::Settings::new();
    settings.set_enable_javascript(false);
    settings.set_enable_javascript_markup(false);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_javascript_can_access_clipboard(false);
    settings.set_enable_html5_database(false);
    settings.set_enable_html5_local_storage(false);
    settings.set_enable_page_cache(false);
    settings.set_enable_media(false);
    settings.set_enable_media_stream(false);
    settings.set_enable_mediasource(false);
    settings.set_enable_encrypted_media(false);
    settings.set_enable_webrtc(false);
    settings.set_enable_webgl(false);
    settings.set_enable_webaudio(false);
    settings.set_enable_fullscreen(false);
    settings.set_enable_developer_extras(cfg!(debug_assertions));
    settings
}

/// Only a user's own click gets to leave the pane. Everything else this
/// signal reports — our own `load_html`, a form submit ammonia already made
/// impossible by stripping `<form>`, a redirect nothing here ever issues —
/// is left to WebKit's normal handling, which for content with nowhere to go
/// is to do nothing.
fn handle_decide_policy(
    view: &webkit6::WebView,
    decision: &webkit6::PolicyDecision,
    kind: webkit6::PolicyDecisionType,
) -> bool {
    if kind == webkit6::PolicyDecisionType::Response {
        return false;
    }
    let Some(navigation) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else {
        return false;
    };
    let Some(mut action) = navigation.navigation_action() else {
        return false;
    };
    if action.navigation_type() != webkit6::NavigationType::LinkClicked {
        return false;
    }
    let Some(uri) = action.request().and_then(|request| request.uri()) else {
        decision.ignore();
        return true;
    };

    let parent = view
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    gtk::UriLauncher::new(&uri).launch(parent.as_ref(), gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            glib::g_warning!("postio", "could not open {}", error);
        }
    });
    decision.ignore();
    true
}

/// Wrap sanitized body markup in the document `load_html` is handed: Postio's
/// own stylesheet, and a `Content-Security-Policy` that closes off anything
/// the sanitizer missed.
///
/// The stylesheet is literal CSS, not the GTK `--postio-*` variables `tokens
/// .css` defines — a `WebView`'s CSS engine has no notion of the GTK style
/// context those live on. `data/reader.css` restates the same values by
/// hand; see its header comment for the values that have to stay in sync.
fn wrap_document(content: &str, remote: RemoteImages) -> String {
    let css = reader_css();
    let csp = content_security_policy(remote);
    format!(
        "<!DOCTYPE html>\n<html><head>\n\
         <meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\n\
         <style>{css}</style>\n\
         </head><body>{content}</body></html>"
    )
}

fn reader_css() -> String {
    let mut css = embedded_font_faces().to_owned();
    if let Ok(bytes) = resources::read(resources::READER_CSS)
        && let Ok(text) = String::from_utf8(bytes.to_vec())
    {
        css.push_str(&text);
    }
    css
}

/// `@font-face` rules embedding the vendored faces as `data:` URIs.
///
/// A `WebView`'s rendering happens in WebKit's own web process, which never
/// sees the `PangoFontMap` [`crate::fonts::install`] populates in this one —
/// referencing the family name by itself would fall back to whatever
/// generic sans the sandbox happens to have. Embedding the bytes is what
/// makes "rendered text inherits Postio typography" true regardless of what
/// the web process can see.
///
/// Computed once: the font bytes are static for the process' life, and
/// base64-encoding four faces on every render would be wasted work.
fn embedded_font_faces() -> &'static str {
    static FACES: OnceLock<String> = OnceLock::new();
    FACES.get_or_init(build_font_faces)
}

fn build_font_faces() -> String {
    const FACES: &[(&str, &str, u16, &str)] = &[
        ("barlow/Barlow-Regular.ttf", "Barlow", 400, "normal"),
        ("barlow/Barlow-Medium.ttf", "Barlow", 500, "normal"),
        ("barlow/Barlow-Bold.ttf", "Barlow", 700, "normal"),
        ("barlow/Barlow-Italic.ttf", "Barlow", 400, "italic"),
        (
            "barlow-condensed/BarlowCondensed-Regular.ttf",
            "Barlow Condensed",
            400,
            "normal",
        ),
        (
            "barlow-condensed/BarlowCondensed-SemiBold.ttf",
            "Barlow Condensed",
            600,
            "normal",
        ),
        (
            "ibm-plex-mono/IBMPlexMono-Regular.ttf",
            "IBM Plex Mono",
            400,
            "normal",
        ),
        (
            "ibm-plex-mono/IBMPlexMono-Medium.ttf",
            "IBM Plex Mono",
            500,
            "normal",
        ),
    ];

    let mut out = String::new();
    for (path, family, weight, style) in FACES {
        let Ok(bytes) = resources::read(&format!("{}/{path}", resources::FONTS)) else {
            continue;
        };
        let base64 = glib::base64_encode(&bytes);
        out.push_str(&format!(
            "@font-face{{font-family:'{family}';font-weight:{weight};\
             font-style:{style};src:url(data:font/ttf;base64,{base64}) format('truetype');}}\n"
        ));
    }
    out
}

/// What the sanitizer already enforces at the DOM level, restated as policy
/// WebKit itself refuses to violate — so a sanitizer bug degrades to broken
/// markup, not a live request.
fn content_security_policy(remote: RemoteImages) -> String {
    let cid = sanitize::CID_SCHEME;
    let img_src = match remote {
        RemoteImages::Blocked => format!("{cid}: data:"),
        RemoteImages::Allowed => format!("{cid}: data: http: https:"),
    };
    format!(
        "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; \
         img-src {img_src}; font-src data:; base-uri 'none'; form-action 'none'; \
         frame-src 'none'; connect-src 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #70, Cause A. Every one of these used to be the same thing on
    /// screen -- a blank pane -- and the pane following the cursor turned
    /// that from an edge case into the common one. Each must now say which
    /// situation it is, because "not downloaded yet" and "the store lost the
    /// body" call for completely different things from the reader.
    #[test]
    fn every_absent_body_says_which_kind_of_absent_it_is() {
        let said: Vec<String> = [
            Absent::Partial,
            Absent::Offline,
            Absent::Missing,
            Absent::Empty,
            Absent::ForeignDraft,
        ]
        .iter()
        .map(|state| absent_html(*state))
        .collect();

        for (state, html) in [
            Absent::Partial,
            Absent::Offline,
            Absent::Missing,
            Absent::Empty,
            Absent::ForeignDraft,
        ]
        .iter()
        .zip(&said)
        {
            assert!(
                !html.trim().is_empty(),
                "{state:?} rendered nothing, which is the bug"
            );
        }

        for (a, first) in said.iter().enumerate() {
            for (b, second) in said.iter().enumerate() {
                assert!(
                    a == b || first != second,
                    "two different reasons produced the same words, so the pane                      cannot be telling the user which one they are looking at"
                );
            }
        }
    }

    /// "Nothing is a dead end": a state the user can act on names the key.
    #[test]
    fn the_states_worth_retrying_name_the_retry_key() {
        // `R` is the registry's own alternate binding for `Refresh`, and the
        // canvas' retry key for the list's empty and error plates. The
        // reading pane must not invent a second one.
        assert!(absent_html(Absent::Offline).contains('R'));
        assert!(absent_html(Absent::Missing).contains('R'));
    }

    /// A message that genuinely has no body is finished, not pending. Telling
    /// someone to retry would be telling them to wait for nothing.
    #[test]
    fn a_message_with_no_body_is_not_offered_a_retry() {
        let html = absent_html(Absent::Empty);
        assert!(!html.contains("check for new mail"), "{html}");
    }

    /// #175: a draft written by another client is a dead end for a different
    /// reason than the other three -- there is nothing to download, because
    /// there is no local buffer to resume. Retrying would promise a fetch
    /// that cannot change the outcome.
    #[test]
    fn a_foreign_draft_says_it_cannot_be_edited_here_and_offers_no_retry() {
        let html = absent_html(Absent::ForeignDraft);
        assert!(!html.contains("check for new mail"), "{html}");
        assert!(
            html.contains("another") || html.contains("device") || html.contains("client"),
            "should say this draft came from somewhere else: {html}"
        );
    }

    #[test]
    fn an_empty_body_produces_empty_content() {
        let (content, blocked) = body_html(&MessageBody::default(), RemoteImages::Blocked);
        assert_eq!(content, "");
        assert_eq!(blocked, 0);
    }

    #[test]
    fn html_is_preferred_over_text_when_both_are_present() {
        let body = MessageBody {
            text: Some("plain fallback".to_owned()),
            html: Some("<p>rich</p>".to_owned()),
        };
        assert_eq!(body_html(&body, RemoteImages::Blocked).0, "<p>rich</p>");
    }

    #[test]
    fn text_only_bodies_still_render() {
        let body = MessageBody {
            text: Some("hello".to_owned()),
            html: None,
        };
        assert!(body_html(&body, RemoteImages::Blocked).0.contains("hello"));
    }

    #[test]
    fn a_remote_image_in_the_body_is_reported_as_blocked() {
        let body = MessageBody {
            text: None,
            html: Some(r#"<img src="https://tracker.example.org/o.gif">"#.to_owned()),
        };
        assert_eq!(body_html(&body, RemoteImages::Blocked).1, 1);
    }

    #[test]
    fn the_csp_only_allows_remote_images_when_asked() {
        assert!(!content_security_policy(RemoteImages::Blocked).contains("https:"));
        assert!(content_security_policy(RemoteImages::Allowed).contains("https:"));
    }

    #[test]
    fn the_document_carries_the_base_uri_and_the_stylesheet() {
        let doc = wrap_document("<p>hi</p>", RemoteImages::Blocked);
        assert!(doc.contains("<style>"));
        assert!(doc.contains("<p>hi</p>"));
        assert!(doc.contains("Content-Security-Policy"));
    }
}
