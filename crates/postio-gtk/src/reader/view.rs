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

use adw::prelude::*;
use gtk::glib;
use postio_model::message::MessageBody;
use webkit6::prelude::*;

use super::actions::ReaderActions;
use super::allowlist::RemoteImageAllowList;
use super::banner::RemoteImageBanner;
use super::message_header::MessageHeader;
use super::scheme::{self, BlobSource};
use postio_body::sanitize::RemoteImages;
// The document itself — CSP, wrapper, fonts, markers, absent states,
// sanitizing and containing the body — is postio-ui's (#567, #590, ADR 0019
// Q6): one implementation for every frontend, re-exported here so existing
// paths keep resolving. What remains in this file is webkit6 glue.
pub use postio_ui::reader::document::{
    Absent, DOCUMENT_BASE_URI, HeldBack, SCROLL_MARKERS, absent_html, body_html,
    content_security_policy, document_for, wrap_document,
};

/// The message currently on screen, kept so the banner's two actions can ask
/// for a re-render without the caller doing it for them.
struct Open {
    body: MessageBody,
    sender: Option<String>,
}

/// Called with how many remote references the pane is currently holding
/// back, every time a render decides that count anew.
type RenderedHandler = Box<dyn Fn(HeldBack)>;

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
    header: Rc<MessageHeader>,
    banner: Rc<RemoteImageBanner>,
    actions: Rc<ReaderActions>,
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
    /// Which of [`SCROLL_MARKERS`]' invisible anchors the pane is currently
    /// at — see [`Reader::page_down`].
    page: Rc<std::cell::Cell<u32>>,
}

/// What [`Reader::connect_parts_requested`] holds.
type PartsRequestedHandler = Box<dyn Fn()>;

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

        let header = Rc::new(MessageHeader::new());
        let banner = Rc::new(RemoteImageBanner::new());
        let actions = ReaderActions::new();

        let chips = crate::parts::Chips::new();

        // The header sits above the banner and does not scroll away with
        // the body (#319): it is a sibling in this native box, never markup
        // inside the `WebView`'s document. The action bar (#498) sits last,
        // under the attachment chips, matching the canvas' footer treatment.
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&header.widget());
        container.append(&banner.widget());
        container.append(&view);
        container.append(&chips.widget());
        container.append(&actions.widget());

        let reader = Reader {
            container,
            view,
            header,
            banner,
            actions,
            allowlist: Rc::new(RefCell::new(allowlist)),
            open: Rc::new(RefCell::new(None)),
            absent: Rc::new(std::cell::Cell::new(None)),
            highlight: Rc::new(RefCell::new(Vec::new())),
            chips,
            rendered: Rc::new(RefCell::new(Vec::new())),
            on_parts_requested: Rc::new(RefCell::new(Vec::new())),
            page: Rc::new(std::cell::Cell::new(0)),
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
            let page = Rc::clone(&reader.page);
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
                        &page,
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
            let page = Rc::clone(&reader.page);
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
                        &page,
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

    /// The message header (#319), for tests that want to assert on its
    /// fields directly rather than parsing the rendered document.
    pub fn header(&self) -> Rc<MessageHeader> {
        Rc::clone(&self.header)
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

    /// Fills in the header (#319): sender, recipients, subject, date — the
    /// three questions a reader asks first, put on screen before the body
    /// even arrives.
    ///
    /// Independent of [`render`](Self::render)/[`show_absent`](Self::show_absent):
    /// the envelope is known as soon as headers have synced, well before a
    /// body might be, so a header-only message gets exactly the same header
    /// a message with a body does.
    pub fn set_message_header(
        &self,
        from: &[postio_model::address::EmailAddress],
        to: &[postio_model::address::EmailAddress],
        cc: &[postio_model::address::EmailAddress],
        subject: Option<&str>,
        date: chrono::DateTime<chrono::Utc>,
    ) {
        self.header.set_message(from, to, cc, subject, date);
    }

    /// Names the account the message on screen arrived in, or hides the line.
    ///
    /// See [`MessageHeader::set_account`] for why the reading pane is where
    /// this is answered and the list row is not (#185).
    pub fn set_account(&self, name: Option<&str>, hue: usize) {
        self.header.set_account(name, hue);
    }

    /// The account line's text, or `None` when hidden. For tests.
    #[doc(hidden)]
    pub fn account_label(&self) -> Option<String> {
        self.header.account_label()
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
        self.actions.set_visible(true);
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
            &self.page,
        );
    }

    /// Called every time a render settles how many remote references are
    /// being held back — see [`RenderedHandler`].
    ///
    /// Fires on the initial render and again whenever the banner's "show
    /// once" or "always allow" changes the count, so a caller wiring the
    /// parts panel's [`crate::parts::PartsPanel::set_held_back`] never goes
    /// stale.
    pub fn connect_rendered(&self, handler: impl Fn(HeldBack) + 'static) {
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

    /// Gives the action bar's buttons the key each currently carries, so a
    /// `[keys]` rebind reaches the pointer's way in the same moment it
    /// reaches the keyboard's. See [`Window::apply_keymap`] for where this is
    /// called from, alongside the finder, the cheat sheet and the parts
    /// panel's own copies.
    ///
    /// [`Window::apply_keymap`]: crate::window::Window::apply_keymap
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        self.actions.set_keymap(keymap);
    }

    /// Called with the invocation whenever a button in the action bar is
    /// pressed — the same [`postio_core::Command`] the keyboard's binding for
    /// the same verb would produce. See
    /// [`crate::list_view::ListView::connect_command`] for the shared shape;
    /// whoever mounts the reader hands this straight to the same
    /// `Window::act` the list's row actions do.
    pub fn connect_command(&self, handler: impl Fn(postio_core::Command) + 'static) {
        self.actions.connect_command(handler);
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
        // A message is still open here — headers arrived, only the body has
        // not — so Reply, Forward and Archive stay reachable exactly as they
        // are from the keyboard while the pane explains why there is no body
        // yet. Only `clear()`'s "nothing selected at all" hides the bar.
        self.actions.set_visible(true);
        self.banner.set_visible(false);
        self.view.load_html(
            &wrap_document(&absent_html(state), RemoteImages::Blocked),
            Some(DOCUMENT_BASE_URI),
        );
        self.page.set(0);
        // No body drawn, so nothing is being held back either — a caller
        // watching `connect_rendered` must not keep showing the previous
        // message's count.
        for handler in self.rendered.borrow().iter() {
            handler(HeldBack::default());
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
        self.header.clear();
        self.actions.set_visible(false);
        self.banner.set_visible(false);
        self.view.load_html(
            &wrap_document("", RemoteImages::Blocked),
            Some(DOCUMENT_BASE_URI),
        );
        self.page.set(0);
        for handler in self.rendered.borrow().iter() {
            handler(HeldBack::default());
        }
    }

    /// Scroll the pane down by about a screenful, without moving the
    /// keyboard off wherever it already is (#438).
    ///
    /// A no-op with nothing open, and at the last marker [`SCROLL_MARKERS`]
    /// lays down — walking past the end of a message is a stop, not a
    /// wrap-around or an error.
    pub fn page_down(&self) {
        if self.open.borrow().is_none() {
            return;
        }
        let next = (self.page.get() + 1).min(SCROLL_MARKERS - 1);
        self.page.set(next);
        self.view
            .load_uri(&format!("{DOCUMENT_BASE_URI}#pos-{next}"));
    }

    /// Scroll the pane up by about a screenful. See [`Reader::page_down`].
    pub fn page_up(&self) {
        if self.open.borrow().is_none() {
            return;
        }
        let previous = self.page.get().saturating_sub(1);
        self.page.set(previous);
        self.view
            .load_uri(&format!("{DOCUMENT_BASE_URI}#pos-{previous}"));
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
    page: &Rc<std::cell::Cell<u32>>,
) {
    let (body, sender) = {
        let guard = open.borrow();
        let Some(current) = guard.as_ref() else {
            return;
        };
        (current.body.clone(), current.sender.clone())
    };
    let (content, held_back) = body_html(&body, remote);
    // After sanitizing and quote-folding, never before: ammonia would strip
    // the `<mark>` as an unknown tag, and there is no point running a matcher
    // over markup that has not been cleaned yet.
    let content = crate::search::mark_html(&content, &highlight.borrow());

    banner.set_sender(sender.as_deref());
    banner.set_visible(remote == RemoteImages::Blocked && held_back.total() > 0);

    let document = document_for(&content, remote);
    view.load_html(&document, Some(DOCUMENT_BASE_URI));
    // `load_html` always starts a document at the top, whatever `page` said
    // before this call -- see `Reader::page_down`.
    page.set(0);

    for handler in rendered.borrow().iter() {
        handler(held_back);
    }
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
    // POSTIO-CONSENT: launched only from a deliberate click on a link in the
    // message the user is reading — this handler fires on a navigation the
    // user started, never on render (the view has JS and network off, so a
    // page cannot navigate itself). Each click is its own consent; nothing
    // is prefetched and no setting turns this on.
    gtk::UriLauncher::new(&uri).launch(parent.as_ref(), gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            glib::g_warning!("postio", "could not open {}", error);
        }
    });
    decision.ignore();
    true
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
