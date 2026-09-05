//! The hardened `WebView`: `postio-lu6`.
//!
//! A message body is hostile input that has to render correctly anyway. The
//! four rules, each backed by a real API rather than a promise:
//!
//! * **JavaScript is off**, at the `WebKitSettings` level, along with every
//!   other scripting-adjacent surface (`WebGL`, `WebRTC`, IndexedDB-style
//!   storage, the offline application cache) — a script disabled by policy in
//!   one place and reachable through another is not disabled.
//! * **Nothing is fetched.** [`postio_body::sanitize_body`] never leaves a
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

use super::allowlist::RemoteImageAllowList;
use super::banner::{DecodeNotice, RemoteImageBanner, UnsubscribeBanner};
use super::message_header::MessageHeader;
use super::scheme::{self, BlobSource};
use crate::widgets::ActionBar;
use postio_body::sanitize::RemoteImages;
// The document itself — CSP, wrapper, fonts, markers, absent states,
// sanitizing and containing the body — is postio-ui's (#567, #590, ADR 0019
// Q6): one implementation for every frontend, re-exported here so existing
// paths keep resolving. What remains in this file is webkit6 glue.
pub use postio_ui::reader::document::{
    Absent, DOCUMENT_BASE_URI, HeldBack, Rendering, SCROLL_MARKERS, Sheet, absent_html, body_html,
    content_security_policy, document_for, reader_ground, sheet_for, wrap_document,
};

/// The message currently on screen, kept so the banner's two actions can ask
/// for a re-render without the caller doing it for them.
struct Open {
    body: MessageBody,
    sender: Option<String>,
    /// Which of the two ways this message is being drawn.
    ///
    /// Per message, and reset by every `render`: `View original` is an answer
    /// about *this* newsletter, and carrying it to the next one would be the
    /// reader quietly deciding that a person who wanted one sender's layout
    /// wants everyone's.
    rendering: Rendering,
    /// Whether reader view had something to offer on this message.
    ///
    /// Recorded at `render` rather than asked again per draw:
    /// [`suits_reader_view`] parses the markup, and the answer cannot change
    /// while one message is on screen. It is what tells `View original` apart
    /// from ordinary correspondence, which is also `Rendering::Original` and
    /// must keep following the theme.
    bulk: bool,
}

/// Called with how many remote references the pane is currently holding
/// back, every time a render decides that count anew.
type RenderedHandler = Box<dyn Fn(HeldBack)>;

/// Called with the list identifier [`Reader::set_unsubscribe`] last set, when
/// the unsubscribe banner's button is activated.
type UnsubscribeHandler = Box<dyn Fn(&str)>;

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
    decode_notice: Rc<DecodeNotice>,
    /// "Reader view — the sender's HTML layout is hidden", with the way
    /// back. Shown only while a message is actually drawn reduced.
    reader_notice: Rc<crate::widgets::NoticeBar>,
    unsubscribe_banner: Rc<UnsubscribeBanner>,
    /// The list identifier [`Reader::set_unsubscribe`] last set — what a
    /// click on the banner's button reports to
    /// [`connect_unsubscribe_activated`](Reader::connect_unsubscribe_activated),
    /// since the click itself carries no data.
    unsubscribe_list: Rc<RefCell<Option<String>>>,
    on_unsubscribe: Rc<RefCell<Vec<UnsubscribeHandler>>>,
    // `ActionBar`, not `ReaderActions`: #1002 replaced the reading pane's
    // hand-rolled bar with the shared one, and `actions.rs` now owns only
    // which four verbs it carries.
    actions: Rc<ActionBar>,
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
    /// How many times the pane has been drawn — see [`Reader::paints`].
    paints: Rc<std::cell::Cell<u32>>,
    /// How many documents have actually been handed to WebKit — see
    /// [`Reader::loads`].
    loads: Rc<std::cell::Cell<u32>>,
    /// The last document handed to WebKit — see [`Reader::test_document`].
    document: Rc<RefCell<String>>,
    /// Set by [`Reader::set_actions_visible`]`(false)` — overrides what
    /// [`render`](Self::render) and [`show_absent`](Self::show_absent) would
    /// otherwise show the action bar for.
    actions_suppressed: Rc<std::cell::Cell<bool>>,
    /// Disconnects this reader's `dark-notify` handler when the last clone
    /// of it goes. See [`DarkNotify`].
    _dark_notify: Rc<DarkNotify>,
}

/// Undoes the one connection a reader makes to something that outlives it.
///
/// `adw::StyleManager::default()` is process-global, and the handler that
/// repaints the pane on a scheme change holds a strong reference to the
/// `WebView`. Connected and never disconnected, that reference is
/// immortal: the view, its `WebContext` — which is a WebProcess — and its
/// `NetworkSession` all survive every drop, for the life of the process.
///
/// In the application that is invisible; there is one reader and it lives
/// as long as the window. In a test binary it is #794: each test builds a
/// reader, none of them ever dies, and at `exit()` WebKit finds the UI
/// process tearing down connections while several WebProcesses are still
/// attached —
///
/// ```text
/// WebProcess didn't exit as expected after the UI process connection
/// was closed
/// ```
///
/// once per leaked view, and then a segfault. Intermittent, because it is a
/// race between exit handlers and processes that should already be gone.
///
/// Held behind an `Rc` rather than implemented as `Drop for Reader`, because
/// `Reader` is `Clone` and every field is a handle: a `Drop` on the struct
/// would disconnect when the *first* clone went out of scope, unhooking a
/// reader that is still on screen.
struct DarkNotify {
    handler: Option<glib::SignalHandlerId>,
}

impl Drop for DarkNotify {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            adw::StyleManager::default().disconnect(handler);
        }
    }
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
        paint_ground(&view);
        // The scheme can change while the application runs, and the widget
        // background is not a document, so no re-render fixes it.
        let dark_notify = adw::StyleManager::default().connect_dark_notify({
            let view = view.clone();
            move |_| paint_ground(&view)
        });

        let header = Rc::new(MessageHeader::new());
        let banner = Rc::new(RemoteImageBanner::new());
        let decode_notice = Rc::new(DecodeNotice::new());
        let reader_notice =
            crate::widgets::NoticeBar::new("view-reveal-symbolic", "postio-reader-view-notice");
        reader_notice.set_text("Reader view — the sender's layout, fonts and footer are hidden");
        reader_notice.set_action(Some("View original"));
        reader_notice.set_action_key(
            postio_core::Keymap::resolve(&Default::default())
                .binding(postio_core::CommandId::ViewOriginal),
        );
        let unsubscribe_banner = Rc::new(UnsubscribeBanner::new());
        let actions = super::actions::new();

        let chips = crate::parts::Chips::new();

        // The header sits above the banner and does not scroll away with
        // the body (#319): it is a sibling in this native box, never markup
        // inside the `WebView`'s document. The action bar (#498) sits last,
        // under the attachment chips, matching the canvas' footer treatment.
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&header.widget());
        container.append(&banner.widget());
        container.append(&decode_notice.widget());
        container.append(&reader_notice.widget());
        container.append(&unsubscribe_banner.widget());
        container.append(&view);
        container.append(&chips.widget());
        container.append(&actions.widget());

        let reader = Reader {
            container,
            view,
            header,
            banner,
            decode_notice,
            reader_notice,
            unsubscribe_banner,
            unsubscribe_list: Rc::new(RefCell::new(None)),
            on_unsubscribe: Rc::new(RefCell::new(Vec::new())),
            actions,
            allowlist: Rc::new(RefCell::new(allowlist)),
            open: Rc::new(RefCell::new(None)),
            absent: Rc::new(std::cell::Cell::new(None)),
            highlight: Rc::new(RefCell::new(Vec::new())),
            chips,
            rendered: Rc::new(RefCell::new(Vec::new())),
            on_parts_requested: Rc::new(RefCell::new(Vec::new())),
            page: Rc::new(std::cell::Cell::new(0)),
            paints: Rc::new(std::cell::Cell::new(0)),
            loads: Rc::new(std::cell::Cell::new(0)),
            document: Rc::new(RefCell::new(String::new())),
            _dark_notify: Rc::new(DarkNotify {
                handler: Some(dark_notify),
            }),
            actions_suppressed: Rc::new(std::cell::Cell::new(false)),
        };

        // The banner's buttons are children of `reader.banner`'s own widget
        // tree, so a closure their "clicked" signal owns must not hold a
        // *strong* `Rc<RemoteImageBanner>` back to it — that would be a
        // button owning (via the signal) a closure owning (via the Rc) the
        // struct that owns the button, a cycle nothing would ever free.
        // `view`, `open` and `allowlist` hold no reference back to the
        // banner, so they can be captured strongly with no such risk.
        // `View original` on the notice runs the same thing `ctrl+o` does.
        // Weakly, for the reason the banner's own buttons are weak: the
        // button lives inside `reader.reader_notice`, so a closure its
        // `clicked` signal owns must not hold a strong reference back to the
        // struct that owns the button.
        {
            let weak = Rc::downgrade(&reader.reader_notice);
            let view = reader.view.clone();
            let open = Rc::clone(&reader.open);
            let allowlist = Rc::clone(&reader.allowlist);
            // Weakly, and this is the half that is easy to get wrong: the
            // banner's own closures hold the notice (below), so a strong
            // reference back would be a cycle between two Rcs that nothing
            // ever frees -- and both of them hold a `WebView` clone, so what
            // leaks is a WebProcess per message. `gtk_reader_teardown` is
            // what says so: "5 of 5 WebViews outlived the readers that made
            // them".
            let banner_from_notice = Rc::downgrade(&reader.banner);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let page = Rc::clone(&reader.page);
            let loads = Rc::clone(&reader.loads);
            let document = Rc::clone(&reader.document);
            reader.reader_notice.connect_action(move || {
                let Some(notice) = weak.upgrade() else { return };
                let Some(banner) = banner_from_notice.upgrade() else {
                    return;
                };
                {
                    let mut guard = open.borrow_mut();
                    let Some(current) = guard.as_mut() else {
                        return;
                    };
                    if current.rendering == Rendering::Original {
                        return;
                    }
                    current.rendering = Rendering::Original;
                }
                let allowed = open
                    .borrow()
                    .as_ref()
                    .and_then(|current| current.sender.clone())
                    .is_some_and(|sender| allowlist.borrow().is_allowed(&sender));
                render_open(
                    &Canvas {
                        view: &view,
                        document: &document,
                        page: &page,
                        loads: &loads,
                    },
                    &banner,
                    &notice,
                    &open,
                    &highlight,
                    if allowed {
                        RemoteImages::Allowed
                    } else {
                        RemoteImages::Blocked
                    },
                    &rendered,
                );
            });
        }

        let banner_weak = Rc::downgrade(&reader.banner);
        {
            let view = reader.view.clone();
            let open = Rc::clone(&reader.open);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let notice_weak = Rc::downgrade(&reader.reader_notice);
            let page = Rc::clone(&reader.page);
            let loads = Rc::clone(&reader.loads);
            let document = Rc::clone(&reader.document);
            let banner_weak = banner_weak.clone();
            reader.banner.connect_show_once(move || {
                let Some(reader_notice) = notice_weak.upgrade() else {
                    return;
                };
                if let Some(banner) = banner_weak.upgrade() {
                    render_open(
                        &Canvas {
                            view: &view,
                            document: &document,
                            page: &page,
                            loads: &loads,
                        },
                        &banner,
                        &reader_notice,
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
            let notice_weak = Rc::downgrade(&reader.reader_notice);
            let page = Rc::clone(&reader.page);
            let loads = Rc::clone(&reader.loads);
            let document = Rc::clone(&reader.document);
            reader.banner.connect_always_allow(move || {
                let Some(reader_notice) = notice_weak.upgrade() else {
                    return;
                };
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
                        &Canvas {
                            view: &view,
                            document: &document,
                            page: &page,
                            loads: &loads,
                        },
                        &banner,
                        &reader_notice,
                        &open,
                        &highlight,
                        RemoteImages::Allowed,
                        &rendered,
                    );
                }
            });
        }

        // No cycle risk here the way the two banner wirings above have to
        // guard against: this closure never calls back into
        // `unsubscribe_banner` itself, only reads `unsubscribe_list` and
        // fires the handlers `connect_unsubscribe_activated` collects.
        {
            let unsubscribe_list = Rc::clone(&reader.unsubscribe_list);
            let on_unsubscribe = Rc::clone(&reader.on_unsubscribe);
            reader.unsubscribe_banner.connect_unsubscribe(move || {
                if let Some(list) = unsubscribe_list.borrow().clone() {
                    for handler in on_unsubscribe.borrow().iter() {
                        handler(&list);
                    }
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

    /// Hide this reader's own action bar regardless of what `render`/
    /// `show_absent` would otherwise show it for, or restore it to following
    /// them again.
    ///
    /// For a reader embedded inside another surface that already draws its
    /// own actions for the same message — the conversation pane's per-entry
    /// row (`crate::conversation::ConversationView::build_entry`) — the same
    /// reason [`Reader::header`]'s identity fields get hidden there. The
    /// surface around this reader already carries Reply/Reply all/Forward;
    /// drawing this reader's own copy on top is a duplicate, not a second
    /// opinion.
    pub fn set_actions_visible(&self, visible: bool) {
        self.actions_suppressed.set(!visible);
        self.actions.set_visible(visible);
    }

    /// Show the action bar unless [`Reader::set_actions_visible`]`(false)`
    /// has suppressed it — what every call site that used to say
    /// `self.actions.set_visible(true)` means now.
    fn show_actions_unless_suppressed(&self) {
        if !self.actions_suppressed.get() {
            self.actions.set_visible(true);
        }
    }

    /// Whether the action bar is currently on screen. For tests.
    #[doc(hidden)]
    pub fn actions_visible(&self) -> bool {
        self.actions.widget().is_visible()
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

    /// Whether the unsubscribe banner is currently on screen.
    pub fn unsubscribe_banner_visible(&self) -> bool {
        self.unsubscribe_banner.is_visible()
    }

    /// The unsubscribe banner's label text — names the list a click would
    /// leave. Test-facing.
    pub fn unsubscribe_banner_label(&self) -> String {
        self.unsubscribe_banner.label()
    }

    /// Simulate clicking "unsubscribe" — what a test uses in place of a
    /// synthesized pointer click.
    pub fn click_unsubscribe(&self) {
        self.unsubscribe_banner.emit_unsubscribe();
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
        self.paints.set(self.paints.get() + 1);
        self.absent.set(None);
        // Cleared here rather than left to the caller. A caveat that outlived
        // the message it was about would be worse than never showing one --
        // it would put "these may not be the sender's words" over mail that
        // decoded perfectly, and a warning that is sometimes wrong is one
        // people learn to ignore. Callers turn it back on for the message
        // they are showing, through `set_encoding_problems`.
        self.decode_notice.set_visible(false);
        self.set_unsubscribe(None);
        // Reader view is decided per message, from the message. Bulk mail
        // opens reduced; correspondence never does. See
        // `document::suits_reader_view` for why the question is "was this
        // laid out by a template" rather than "could this be reduced".
        let bulk = postio_ui::reader::document::suits_reader_view(body);
        let rendering = if bulk {
            Rendering::Reader
        } else {
            Rendering::Original
        };
        *self.open.borrow_mut() = Some(Open {
            body: body.clone(),
            sender: sender.map(str::to_owned),
            rendering,
            bulk,
        });
        self.show_actions_unless_suppressed();
        let allowed = sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(sender));
        let remote = if allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        render_open(
            &self.canvas(),
            &self.banner,
            &self.reader_notice,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
    }

    /// Draw the sender's own markup for whatever is on screen — `C-o`.
    ///
    /// Per message and not sticky: the next message decides for itself. A
    /// person who wanted to see one newsletter's layout has not said anything
    /// about the next one, and a reader that remembered would be answering a
    /// question nobody asked.
    ///
    /// A no-op when the pane is empty or already showing the original, so the
    /// key is safe to press anywhere.
    pub fn view_original(&self) {
        {
            let mut guard = self.open.borrow_mut();
            let Some(open) = guard.as_mut() else { return };
            if open.rendering == Rendering::Original {
                return;
            }
            open.rendering = Rendering::Original;
        }
        self.rerender();
    }

    /// Whether the pane is currently drawing a message reduced.
    ///
    /// The drawn state, not an intention: a test asking "is reader view on"
    /// wants to know what a person can see.
    pub fn is_reader_view(&self) -> bool {
        self.open
            .borrow()
            .as_ref()
            .is_some_and(|open| open.rendering == Rendering::Reader)
    }

    /// Whether the notice offering `View original` is on screen.
    pub fn reader_notice_visible(&self) -> bool {
        self.reader_notice.is_visible()
    }

    /// Press `View original` without a pointer, for a test.
    pub fn click_view_original(&self) {
        self.reader_notice.press_action();
    }

    /// Draw whatever is open again, with its current rendering.
    ///
    /// What `View original` needs and `render` cannot give it: the message
    /// has not changed, only the way it is being drawn, so re-deriving the
    /// remote-image decision and re-entering `render` would reset the very
    /// choice that was just made.
    fn rerender(&self) {
        let allowed = self
            .open
            .borrow()
            .as_ref()
            .and_then(|open| open.sender.clone())
            .is_some_and(|sender| self.allowlist.borrow().is_allowed(&sender));
        let remote = if allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        render_open(
            &self.canvas(),
            &self.banner,
            &self.reader_notice,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
    }

    /// The `WebView` and its load bookkeeping, together — see [`Canvas`].
    fn canvas(&self) -> Canvas<'_> {
        Canvas {
            view: &self.view,
            document: &self.document,
            page: &self.page,
            loads: &self.loads,
        }
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
        // The notice's own cap, from the same keymap. Written down here it
        // would go on saying `C-o` after a rebind moved the key, which is the
        // drift `KeycapButton` exists to end (#1002).
        self.reader_notice
            .set_action_key(keymap.binding(postio_core::CommandId::ViewOriginal));
    }

    /// Called with the invocation whenever a button in the action bar is
    /// pressed — the same [`postio_core::Command`] the keyboard's binding for
    /// the same verb would produce. See
    /// [`crate::list_view::MessageListView::connect_command`] for the shared shape;
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
        self.paints.set(self.paints.get() + 1);
        *self.open.borrow_mut() = None;
        self.absent.set(Some(state));
        // A message is still open here — headers arrived, only the body has
        // not — so Reply, Forward and Archive stay reachable exactly as they
        // are from the keyboard while the pane explains why there is no body
        // yet. Only `clear()`'s "nothing selected at all" hides the bar.
        self.show_actions_unless_suppressed();
        self.banner.set_visible(false);
        load_document(
            &self.canvas(),
            &wrap_document(&absent_html(state), RemoteImages::Blocked, Sheet::Theme),
        );
        // No body drawn, so nothing is being held back either — a caller
        // watching `connect_rendered` must not keep showing the previous
        // message's count.
        for handler in self.rendered.borrow().iter() {
            handler(HeldBack::default());
        }
    }

    /// How many times this pane has been asked to draw a message — a body
    /// through [`render`](Self::render), or a plate through
    /// [`show_absent`](Self::show_absent).
    ///
    /// Test-facing, and the only way to tell a repaint that was coalesced
    /// from one that was merely idempotent: twenty arrivals for the message
    /// on screen and twenty repaints look identical in every other
    /// observable, and the difference is a sync spent redrawing (#396).
    #[doc(hidden)]
    pub fn paints(&self) -> u32 {
        self.paints.get()
    }

    /// How many documents this pane has actually handed to WebKit.
    ///
    /// [`paints`](Self::paints) counts times the pane was *asked* to draw;
    /// this counts the times that cost a document teardown and reload. The
    /// two differ exactly where #749 lives: an arrival that recomposes the
    /// document byte-for-byte identically is a paint that must not be a
    /// load, because every load is a black frame's worth of unpainted
    /// `WebView` and a scroll position thrown away.
    #[doc(hidden)]
    pub fn loads(&self) -> u32 {
        self.loads.get()
    }

    /// The document the pane last handed to WebKit.
    ///
    /// The reader's `WebView` runs with JavaScript off, so a test cannot ask
    /// the live page what it painted; this is the finished document, which is
    /// the last thing that exists before WebKit and the place a wiring
    /// mistake shows. Not meant for anything but tests.
    #[doc(hidden)]
    pub fn test_document(&self) -> String {
        self.document.borrow().clone()
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
    /// Say whether the body on screen is a guess rather than what was sent.
    ///
    /// The end of the road for `ParsedMessage::encoding_problems`, which was
    /// computed and read by nothing (#901): base64 outside its alphabet
    /// arriving as raw base64 text, an unknown `Content-Transfer-Encoding`
    /// shown verbatim per RFC 2045 §6.4, a charset that lost octets to
    /// U+FFFD. Each is a defensible degradation and each is indistinguishable
    /// from a message that simply said that, which is the same failure as
    /// #70's blank column: "nothing rendered" and "nothing was there" are
    /// opposite facts that looked identical.
    ///
    /// Call it after [`render`](Self::render), which clears it.
    pub fn set_encoding_problems(&self, problems: bool) {
        self.decode_notice.set_visible(problems);
    }

    /// Whether the decode caveat is on screen.
    pub fn shows_encoding_problems(&self) -> bool {
        self.decode_notice.is_visible()
    }

    /// Name the list this message belongs to, or say it belongs to none.
    ///
    /// `#971`: `list_identifier` is a `List-Id` header when the message had
    /// one, or the sender's domain otherwise — whichever the caller found;
    /// this only shows what it is handed. Call it after
    /// [`render`](Self::render), which clears it, same convention as
    /// [`set_encoding_problems`](Self::set_encoding_problems).
    pub fn set_unsubscribe(&self, list_identifier: Option<&str>) {
        *self.unsubscribe_list.borrow_mut() = list_identifier.map(str::to_owned);
        self.unsubscribe_banner.set_list(list_identifier);
    }

    /// Called with the list identifier when the unsubscribe banner's button
    /// is activated — the reader only asks; a caller decides what leaving a
    /// list means (`postio-gtk` has no SQL to log the activation itself).
    pub fn connect_unsubscribe_activated(&self, handler: impl Fn(&str) + 'static) {
        self.on_unsubscribe.borrow_mut().push(Box::new(handler));
    }

    pub fn clear(&self) {
        *self.open.borrow_mut() = None;
        self.absent.set(None);
        self.header.clear();
        self.actions.set_visible(false);
        self.banner.set_visible(false);
        self.decode_notice.set_visible(false);
        self.set_unsubscribe(None);
        load_document(
            &self.canvas(),
            &wrap_document("", RemoteImages::Blocked, Sheet::Theme),
        );
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

/// Give the `WebView` an opaque background of its own, matching the ground
/// the document will paint.
///
/// **Why a widget needs a colour when its document already has one.** Every
/// message change here is a full document teardown and reload — JavaScript
/// is off, so there is no incremental path — and `reader.css`'s
/// `body { background: var(--r-ground) }` cannot apply until the *new*
/// document has parsed. In between, the view paints its own background, and
/// WebKit's default under the GTK4 GL/DMA-BUF path shows as black. That is
/// the flash #749 reported: not a state Postio draws, but the absence of one.
///
/// Setting it does not make the reload free — the fix for that is not
/// reloading when nothing changed, which [`load_document`] does — but it
/// makes the gap invisible instead of a black frame.
fn paint_ground(view: &webkit6::WebView) {
    let dark = adw::StyleManager::default().is_dark();
    match reader_ground(dark).parse::<gtk::gdk::RGBA>() {
        Ok(ground) => view.set_background_color(&ground),
        // Not fatal: an unpainted view is the state this improves on, not one
        // it depends on. Worth saying out loud, though — it means the
        // generated palette stopped being something gdk can parse.
        Err(error) => glib::g_warning!(
            "postio",
            "could not parse the reader ground colour: {error}"
        ),
    }
}

/// The `WebView` and the bookkeeping that decides whether it needs a new
/// document at all.
///
/// Bundled rather than passed as four more arguments because every caller
/// needs all four together: loading a document is exactly the moment the
/// scroll anchor resets, the tally moves, and the hash of what is on screen
/// changes. Splitting them has already gone wrong once — `render_open` reset
/// `page` whether or not the load was worth doing.
struct Canvas<'a> {
    view: &'a webkit6::WebView,
    /// The last document handed to WebKit, kept for [`Reader::test_document`].
    ///
    /// The reader's `WebView` has JavaScript off by construction, so a test
    /// cannot ask the live page what colour it ended up painting -- the one
    /// assertion that would be closer to what a person sees is the one this
    /// pane's hardening rules out. This is the next thing down: the finished
    /// document, the last artifact before WebKit, which is where a wiring
    /// mistake would show.
    document: &'a RefCell<String>,
    /// Which of [`SCROLL_MARKERS`]' anchors the pane is at.
    page: &'a Rc<std::cell::Cell<u32>>,
    /// Documents actually handed to WebKit — [`Reader::loads`].
    loads: &'a Rc<std::cell::Cell<u32>>,
}

/// Hand `document` to WebKit, and count it.
///
/// Every call here is a full document teardown and reload: JavaScript is off,
/// so there is no incremental path, and the page cache is off too. Between
/// the old document being discarded and the new one's first paint the
/// `WebView` has nothing of its own to draw — the black frame #749 reported,
/// which [`paint_ground`] covers — and the reader's scroll position is gone.
///
/// So a load is a cost, and the tally is the observable a test can hold it to.
/// Deciding whether a load is *needed* is deliberately not done here: this
/// pane is handed a body and a sender, not a message, so it cannot tell a
/// second message that happens to compose an identical document from the same
/// message arriving twice — and those two want opposite answers. That
/// judgement belongs where message identity exists, in `postio_app::reading`.
fn load_document(canvas: &Canvas<'_>, document: &str) {
    canvas.loads.set(canvas.loads.get() + 1);
    canvas.document.replace(document.to_owned());
    canvas.view.load_html(document, Some(DOCUMENT_BASE_URI));
    // `load_html` always starts a document at the top, whatever `page` said
    // before this call -- see `Reader::page_down`.
    canvas.page.set(0);
}

/// Re-render whatever is in `open` at `remote`'s policy, and put the banner
/// in step with the result.
///
/// A free function, not a method, so the two banner-signal closures can call
/// it through weak/`Rc` captures without holding a whole `Reader` (which
/// would capture `container` — and so the banner and the button doing the
/// capturing — in a reference cycle nothing would ever free).
fn render_open(
    canvas: &Canvas<'_>,
    banner: &RemoteImageBanner,
    reader_notice: &crate::widgets::NoticeBar,
    open: &Rc<RefCell<Option<Open>>>,
    highlight: &Rc<RefCell<Vec<String>>>,
    remote: RemoteImages,
    rendered: &Rc<RefCell<Vec<RenderedHandler>>>,
) {
    let (body, sender, rendering, bulk) = {
        let guard = open.borrow();
        let Some(current) = guard.as_ref() else {
            return;
        };
        (
            current.body.clone(),
            current.sender.clone(),
            current.rendering,
            current.bulk,
        )
    };
    let drawn = body_html(&body, remote, rendering);
    let held_back = drawn.held_back;
    let content = drawn.html.clone();
    // After sanitizing and quote-folding, never before: ammonia would strip
    // the `<mark>` as an unknown tag, and there is no point running a matcher
    // over markup that has not been cleaned yet.
    let content = crate::search::mark_html(&content, &highlight.borrow());

    banner.set_sender(sender.as_deref());
    // The count, before the visibility: a notice that appeared and then
    // changed what it said would flicker a number at the reader (#1008).
    banner.set_held_back(held_back);
    banner.set_visible(remote == RemoteImages::Blocked && held_back.total() > 0);

    // Only while a message is actually drawn reduced. A notice offering to
    // show an original that is already on screen is a control that does
    // nothing, which is worse than no control.
    reader_notice.set_visible(drawn.rendering == Rendering::Reader);
    if drawn.rendering == Rendering::Reader && drawn.links_dropped > 0 {
        reader_notice.set_text(&format!(
            "Reader view — {} link{} kept of {}",
            drawn.links_kept,
            if drawn.links_kept == 1 { "" } else { "s" },
            drawn.links_total()
        ));
    } else {
        reader_notice.set_text("Reader view — the sender's layout, fonts and footer are hidden");
    }

    // Which paper this goes on. `Rendering::Original` alone is not enough --
    // correspondence is Original too, and must keep following the theme; the
    // sender's sheet is for the person who left reader view to see what was
    // actually sent. `sheet_for` is where that rule lives, so this frontend
    // and the FFI one cannot express it differently.
    let sheet = sheet_for(drawn.rendering, bulk);
    load_document(canvas, &document_for(&content, remote, sheet));

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
/// Whether a navigation is one the pane hands to the desktop rather than
/// following itself.
///
/// Exactly one kind is: a link the person reading deliberately clicked.
/// Everything else — our own `load_html` (which arrives as
/// [`NavigationType::Other`]), a fragment jump from `Reader::page_down`, a
/// form submission, a reload — either is the pane doing its own job or is
/// something a message must not be able to start. A predicate rather than an
/// inline condition because it is the whole of the rule, and because the two
/// enums are plain values: this is checkable without driving WebKit to a
/// navigation, which nothing else here can do with JavaScript off.
///
/// [`NavigationType::Other`]: webkit6::NavigationType::Other
fn leaves_the_pane(kind: webkit6::PolicyDecisionType, navigation: webkit6::NavigationType) -> bool {
    kind != webkit6::PolicyDecisionType::Response
        && navigation == webkit6::NavigationType::LinkClicked
}

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
    if !leaves_the_pane(kind, action.navigation_type()) {
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

    /// #752: a clicked link goes to the desktop, and nothing else does.
    ///
    /// The second half is the load-bearing one. The pane renders by calling
    /// `load_html`, which comes back through this same signal — so a rule
    /// that intercepted more than `LinkClicked` would take the reader's own
    /// documents away from it, and one that intercepted less would let a
    /// message navigate the pane out from under the person reading it.
    #[test]
    fn only_a_clicked_link_leaves_the_reading_pane() {
        assert!(leaves_the_pane(
            webkit6::PolicyDecisionType::NavigationAction,
            webkit6::NavigationType::LinkClicked,
        ));
        for navigation in [
            // Our own `load_html`, and a `page_down` fragment jump.
            webkit6::NavigationType::Other,
            webkit6::NavigationType::FormSubmitted,
            webkit6::NavigationType::BackForward,
            webkit6::NavigationType::Reload,
            webkit6::NavigationType::FormResubmitted,
        ] {
            assert!(
                !leaves_the_pane(webkit6::PolicyDecisionType::NavigationAction, navigation),
                "{navigation:?} is the pane's own business, not the desktop's"
            );
        }
        assert!(
            !leaves_the_pane(
                webkit6::PolicyDecisionType::Response,
                webkit6::NavigationType::LinkClicked,
            ),
            "a response decision is not a navigation to hand off"
        );
    }

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
        let doc = wrap_document("<p>hi</p>", RemoteImages::Blocked, Sheet::Theme);
        assert!(doc.contains("<style>"));
        assert!(doc.contains("<p>hi</p>"));
        assert!(doc.contains("Content-Security-Policy"));
    }
}
