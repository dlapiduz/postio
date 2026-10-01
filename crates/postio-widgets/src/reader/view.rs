//! The reading pane: `postio-lu6`, drawn by the reading renderer (spec 006).
//!
//! A message body is hostile input that has to render correctly anyway. The
//! rules, each held by construction rather than by a setting:
//!
//! * **No script runs.** The body is drawn by `postio-render`, which has no
//!   script engine at all; nothing in a message can execute, and nothing of
//!   Postio's needs to, because the rail and the scroll are read from the
//!   snapshot's geometry.
//! * **Nothing is fetched.** [`postio_body::sanitize_body`] never leaves a
//!   remote reference in the markup unless the user allowed it
//!   (`postio-xxz`), and the renderer resolves only what it is handed: a
//!   message's own parts, Postio's faces, and remote images the reader's
//!   owner fetched on consent ([`Reader::set_remote_fetch`]).
//! * **Inline images stay local.** `cid:` references resolve through
//!   whatever [`BlobSource`] the caller hands in -- a blob-store read, never
//!   a network round trip.
//! * **A click never navigates the pane.** The view resolves a link from
//!   the snapshot: a sender's link goes to [`gtk::UriLauncher`] on a
//!   deliberate click, a message's own verb comes back here, and a fragment
//!   scrolls.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_model::message::MessageBody;

use super::BlobSource;
use super::banner::{DecodeNotice, RemoteImageBanner, UnsubscribeBanner};
use super::message_header::MessageHeader;
use super::notices::{Notice, NoticeSlot};
use crate::widgets::ActionBar;
use postio_body::sanitize::RemoteImages;
use postio_ui::allowlist::RemoteImageAllowList;
// The document itself — CSP, wrapper, fonts, markers, absent states,
// sanitizing and containing the body — is postio-ui's (#567, #590, ADR 0019
// Q6): one implementation for every frontend, re-exported here so existing
// paths keep resolving.
pub use postio_body::treatment::Treatment;
pub use postio_ui::reader::document::{
    Absent, HeldBack, Rendering, Sheet, Treated, absent_html, body_html, content_security_policy,
    document_for, reader_ground, sheet_for, wrap_document,
};

use super::render_mode::RenderModeLine;

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
    /// [`postio_ui::reader::document::suits_reader_view`] parses the markup,
    /// and the answer cannot change
    /// while one message is on screen. It is what tells `View original` apart
    /// from ordinary correspondence, which is also `Rendering::Original` and
    /// must keep following the theme.
    bulk: bool,
    /// The body already judged and sanitised off the main thread, when the
    /// caller had it done there (`Reader::render_prepared`). Used only while
    /// it serves exactly this body, policy and rendering; a banner or notice
    /// that changes either is drawn as before.
    prepared: Option<postio_ui::reader::document::Prepared>,
    /// The treatment the person chose for this message -- `O`, or what they
    /// remembered for its sender -- or `None` for the rule's
    /// (specs/007-postio-focus T213). Read only by a reader that draws
    /// treatments ([`Reader::use_treatments`]).
    chosen: Option<Treatment>,
    /// What is remembered for this message's sender, for the line to say.
    remembered: Option<Treatment>,
}

/// Called with how many remote references the pane is currently holding
/// back, every time a render decides that count anew.
type RenderedHandler = Box<dyn Fn(HeldBack)>;

/// Called with the list identifier [`Reader::set_unsubscribe`] last set, when
/// the unsubscribe banner's button is activated.
type UnsubscribeHandler = Box<dyn Fn(&str)>;

/// The reading pane: the body view, and the header, notices and banners
/// (`postio-xxz`) that sit above it.
///
/// `Clone` is cheap — every field is a GObject reference or an `Rc` — so a
/// caller can hand a `Reader` to more than one closure without fighting the
/// borrow checker over who owns it.
#[derive(Clone)]
pub struct Reader {
    container: gtk::Box,
    /// What a surface puts under the header: Focus's marker card. Hidden
    /// while empty, so the notices and the body start where they did.
    under_header: gtk::Box,
    view: crate::body_view::BodyView,
    /// The view's own scroller, and the overlay it sits in: what
    /// [`Reader::flow_in`] takes the view out of.
    scroller: gtk::ScrolledWindow,
    body: gtk::Overlay,
    /// Find in the message (spec 006 FR-018), above the body.
    find: Rc<crate::body_view::find::FindBar>,
    /// The zoom, when it is not actual size, over the body's corner.
    zoom_indicator: Rc<crate::body_view::zoom::ZoomIndicator>,
    header: Rc<MessageHeader>,
    banner: Rc<RemoteImageBanner>,
    /// "Reader view — the sender's HTML layout is hidden", with the way
    /// back. Shown only while a message is actually drawn reduced.
    reader_notice: Rc<crate::widgets::NoticeBar>,
    unsubscribe_banner: Rc<UnsubscribeBanner>,
    /// The one slot the four notices above share, so the body under them
    /// starts at the same place whichever of them apply. See
    /// [`super::notices`].
    notices: Rc<NoticeSlot>,
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
    /// The bar a message waiting to be sent gets, and the one a stopped send
    /// gets. See [`Reader::set_send_state`].
    queued_actions: Rc<ActionBar>,
    stopped_actions: Rc<ActionBar>,
    /// What the message on screen is doing, so `render` can put the right
    /// bar back after clearing.
    send_state: Rc<std::cell::Cell<Option<postio_model::DraftState>>>,
    /// Whether a message occupies the pane at all.
    ///
    /// Separate from [`Self::send_state`], and from `actions_suppressed`,
    /// because they answer different questions: *which* verbs, *whether the
    /// surface allows any*, and *is there anything to act on*. Without this
    /// third one, picking the verbs for "no send state" showed the ordinary
    /// bar over an empty pane.
    showing: Rc<std::cell::Cell<bool>>,
    allowlist: Rc<RefCell<RemoteImageAllowList>>,
    /// The thread currently drawn, so a `Show` verb inside the document can
    /// find the message its scope names and the sender that message is from.
    /// Empty whenever a single message is drawn instead.
    thread: Rc<RefCell<Vec<ThreadMessage>>>,
    /// The messages of the open thread the reader has been asked to show
    /// whole, by scope.
    ///
    /// How the reader asked for particular messages of the thread to be
    /// drawn: `⌃O` for the sender's own markup, reader view for reduced.
    /// Every other message opens as [`opening_rendering`] says (spec 006
    /// FR-031). Kept beside the thread rather than inside `Open`, which only
    /// the single-message path fills — reading `Open` is what made `⌃O` a
    /// no-op here (#1398).
    ///
    /// [`opening_rendering`]: postio_ui::reader::document::opening_rendering
    originals: Rc<RefCell<std::collections::HashMap<String, Rendering>>>,
    /// What the sanitiser made of each message of the thread on screen, so a
    /// redraw re-sanitises only what changed (#1605).
    renders: Rc<RefCell<postio_ui::reader::document::RenderCache>>,
    /// Who to tell when a message's own verb is activated.
    on_message_action: Rc<RefCell<Vec<MessageActionHandler>>>,
    /// Who to tell when the message filling the pane changes.
    on_current_message: Rc<RefCell<Vec<CurrentMessageHandler>>>,
    open: Rc<RefCell<Option<Open>>>,
    /// Which [`Absent`] the pane is explaining, when it has no body to draw.
    /// `None` whenever a body is on screen — the two are exclusive, and
    /// `render` and `clear` both say so.
    absent: Rc<std::cell::Cell<Option<Absent>>>,
    /// Terms to paint where they appear in the body. Empty for ordinary
    /// reading; set while a search is what put the message on screen.
    highlight: Rc<RefCell<Vec<String>>>,
    /// What came with the message, per canvas 1b — and the way into the
    /// parts panel. See [`super::chips::Chips`].
    chips: super::chips::Chips,
    /// Called every time a render settles how many remote references are
    /// being held back — initial render, and again if the banner's "show
    /// once" or "always allow" changes it. See [`connect_rendered`].
    ///
    /// [`connect_rendered`]: Reader::connect_rendered
    rendered: Rc<RefCell<Vec<RenderedHandler>>>,
    /// Called when `p` asks to see the parts panel for whatever is showing,
    /// with no chip to click — see [`Reader::connect_parts_requested`].
    on_parts_requested: Rc<RefCell<Vec<PartsRequestedHandler>>>,
    /// How many times the pane has been drawn — see [`Reader::paints`].
    paints: Rc<std::cell::Cell<u32>>,
    /// How many documents have actually been handed to the view — see
    /// [`Reader::loads`].
    loads: Rc<std::cell::Cell<u32>>,
    /// The last document handed to the view — see [`Reader::test_document`].
    document: Rc<RefCell<String>>,
    /// Where the reader is in the document, so a redraw of the same
    /// content can put them back there. See [`Place`].
    place: Rc<Place>,
    /// Where a change to the allow list -- an "Always allow", an "Always
    /// for this sender" -- is saved.
    allowlist_path: Rc<std::path::PathBuf>,
    /// Set by [`Reader::set_actions_visible`]`(false)` — overrides what
    /// [`render`](Self::render) and [`show_absent`](Self::show_absent) would
    /// otherwise show the action bar for.
    actions_suppressed: Rc<std::cell::Cell<bool>>,
}

impl Drop for Reader {
    /// Balances [`postio_ui::reader::cost::note_surface_created`] so that
    /// `surfaces_held` means what it says.
    ///
    /// Dropping the `Reader` is what lets its body view go, and the render
    /// thread and snapshot with it. A conversation that keeps every surface
    /// it ever opened is the defect ADR 0032 describes, and it is invisible
    /// to any count that only watches creations.
    fn drop(&mut self) {
        postio_ui::reader::cost::note_surface_released();
    }
}

/// What [`Reader::connect_current_message`] holds: the scope of the message
/// filling most of the pane.
type CurrentMessageHandler = Box<dyn Fn(&str)>;

/// What [`Reader::connect_message_action`] holds: the scope a verb named, and
/// which verb it was.
type MessageActionHandler = Box<dyn Fn(&str, MessageVerb)>;

/// A verb a message offers for itself, inside the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageVerb {
    /// Reply to this message rather than to the latest one.
    Reply,
    /// Forward this message.
    Forward,
    /// Resume the composer on this draft (#1212).
    ///
    /// The verb a draft offers instead of the other two, and the same command
    /// activating the row raises -- `CommandId::OpenMessage` -- so a button
    /// and `Return` cannot come to mean different things.
    Continue,
}

/// What [`Reader::connect_parts_requested`] holds.
type PartsRequestedHandler = Box<dyn Fn()>;

/// One message's place in a conversation rendered into a single view.
///
/// `Clone` because the reader keeps the thread it drew: the `Show` verb inside
/// the document has to re-render after granting consent, and it re-renders the
/// same messages rather than asking the application for them again.
///
/// See [`Reader::render_thread`] and ADR 0032.
#[derive(Clone)]
pub struct ThreadMessage {
    /// What this message's `cid:` references are stamped with, and what the
    /// scheme handler routes on. The message id in decimal: unreserved
    /// characters only, since it goes into a URI unescaped.
    pub scope: String,
    /// Who it is from, as a person reads it.
    pub sender: String,
    /// Their address, shown beside the name on an open message (canvas 17).
    pub address: String,
    /// When, already formatted.
    pub when: String,
    /// Who it went to, already drawn by
    /// `postio_ui::reader::header::recipient_line` -- the same rule the
    /// stacked pane's per-entry header uses, so the two panes cannot start
    /// counting recipients differently (#1427).
    pub recipients: String,
    /// Who else was copied, by the same rule. Empty when nobody was.
    pub cc: String,
    /// The one line a collapsed message shows.
    pub preview: String,
    /// Whether it starts open.
    pub expanded: bool,
    /// Whether the body has not been backfilled yet.
    ///
    /// A conversation is one document (ADR 0032), so a message with no body
    /// used to contribute an empty section and say nothing -- the reader saw
    /// a message that would not open and no reason why. `Absent::Partial`'s
    /// plate is the answer the single-message path has always given, and this
    /// is what carries the question into the thread.
    ///
    /// Only the message that is *open* shows the plate. Everything unfetched
    /// stays the one line it already was, because
    /// `expanded_in_document` opens every message in the thread and a plate
    /// on each would be thirty explanations of one fact.
    pub absent: bool,
    /// Whether this is the newest message in the thread — canvas 17's badge.
    pub latest: bool,
    /// Whether this is a draft: written here and never sent.
    ///
    /// Changes which verbs the message offers -- `Continue editing` alone
    /// (#1212). The conversation knows it from `Row::draft`; the reader only
    /// carries it through to the document.
    pub draft: bool,
    /// Whether it came from one of the account's own addresses (#1241).
    ///
    /// Folded by the conversation, which is the only layer that knows the
    /// account's identities -- `postio-gtk`'s reader has no notion of who
    /// the user is and should not grow one.
    pub mine: bool,
    /// The message body, unsanitised — [`Reader::render_thread`] sanitises it
    /// under [`scope`](Self::scope), which is the only way the reference
    /// stamping can be guaranteed.
    pub body: MessageBody,
}

/// Keeping the reader's place, and what a load needs besides the markup.
///
/// A redraw of the same content in front of the same person -- a late body
/// arriving, a message redrawn to show its images -- keeps where they were:
/// the view keeps its scroll across a snapshot. A different message starts
/// at the top. `keep` says which the next load is.
///
/// It also carries what `load_document` needs that `Canvas` never did:
/// where parts come from, and the plain text a render falls back to. Never
/// the view itself: the view's own signal handlers hold a `Place`, and a
/// `Place` holding the view back would be a cycle that keeps every reader's
/// renderer alive for the life of the process.
struct Place {
    source: Rc<dyn BlobSource>,
    /// The message's plain-text alternative, shown if a render falls back.
    plain: RefCell<String>,
    keep: std::cell::Cell<bool>,
    /// The document on screen, so a late arrival redraws only that one.
    shown: RefCell<String>,
    /// What fetches a document's remote images: the reader's owner's, and
    /// nothing until it sets one. This crate speaks no protocol.
    fetch: RefCell<Option<Rc<RemoteFetch>>>,
    /// Remote images that arrived, by URL, for this reader's life.
    remote: RefCell<std::collections::HashMap<String, Vec<u8>>>,
    /// URLs already asked for, arrived or not: each is asked once.
    asked: RefCell<std::collections::HashSet<String>>,
    /// The next load follows the user's explicit consent -- "Show once",
    /// "Always allow", a message's own Show -- so its images are asked for
    /// at once rather than after the dwell.
    consented: std::cell::Cell<bool>,
    /// The reader flows in its owner's column ([`Reader::flow_in`]).
    flow: std::cell::Cell<bool>,
    /// Bodies are drawn under a treatment ([`Reader::use_treatments`]).
    treatments: std::cell::Cell<bool>,
    /// How the body on screen was treated, when it was.
    treated: std::cell::Cell<Option<Treated>>,
    /// The line naming it, once [`Reader::use_treatments`] made one.
    line: RefCell<Option<Rc<RenderModeLine>>>,
    /// Told the treatment on screen each time a body is drawn under one.
    on_treatment: RefCell<Vec<TreatmentHandler>>,
}

/// Called with the treatment the body on screen is drawn in.
type TreatmentHandler = Rc<dyn Fn(Treatment)>;

/// What a reader's owner fetches remote images with: the URLs one document
/// names as images, and where to hand what arrived, on the main thread.
pub type RemoteFetch = dyn Fn(Vec<String>, RemoteArrived);

/// The images that arrived for one request, by URL.
pub type RemoteArrived = Box<dyn FnOnce(Vec<(String, Vec<u8>)>)>;

impl Place {
    fn new(source: Rc<dyn BlobSource>) -> Self {
        Place {
            source,
            plain: RefCell::new(String::new()),
            keep: std::cell::Cell::new(false),
            shown: RefCell::new(String::new()),
            fetch: RefCell::new(None),
            remote: RefCell::default(),
            asked: RefCell::default(),
            consented: std::cell::Cell::new(false),
            flow: std::cell::Cell::new(false),
            treatments: std::cell::Cell::new(false),
            treated: std::cell::Cell::new(None),
            line: RefCell::new(None),
            on_treatment: RefCell::new(Vec::new()),
        }
    }
}

impl Reader {
    /// Build a reader that resolves inline (`cid:`) images through `source`.
    ///
    /// The remote-image allow list is the one every reader of the app
    /// shares ([`super::shared_allowlist`]), persisted in `$XDG_STATE_HOME`:
    /// loaded once, so no reader can disagree with another about who is
    /// allowed.
    pub fn new(source: Rc<dyn BlobSource>) -> Self {
        Self::sharing(
            source,
            &RemoteImageAllowList::path(),
            super::Verbs::STANDARD,
        )
    }

    /// A reader on the allow list this app's other readers persisting to
    /// `allowlist_path` share ([`super::shared_allowlist`]), drawing `verbs`:
    /// one "Always allow" or revoke reaches all of them (T020).
    pub fn sharing(
        source: Rc<dyn BlobSource>,
        allowlist_path: &std::path::Path,
        verbs: super::Verbs,
    ) -> Self {
        Self::build(
            source,
            super::shared_allowlist(allowlist_path),
            allowlist_path.to_owned(),
            verbs,
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
        Self::with_verbs(source, allowlist, allowlist_path, super::Verbs::STANDARD)
    }

    /// As [`with_allowlist`](Self::with_allowlist), drawing `verbs` in the
    /// header: the reading pane's [`Verbs::STANDARD`](super::Verbs::STANDARD),
    /// or [`Verbs::NONE`](super::Verbs::NONE) for a surface whose own
    /// toolbar carries them.
    pub fn with_verbs(
        source: Rc<dyn BlobSource>,
        allowlist: RemoteImageAllowList,
        allowlist_path: std::path::PathBuf,
        verbs: super::Verbs,
    ) -> Self {
        Self::build(
            source,
            Rc::new(RefCell::new(allowlist)),
            allowlist_path,
            verbs,
        )
    }

    fn build(
        source: Rc<dyn BlobSource>,
        allowlist: Rc<RefCell<RemoteImageAllowList>>,
        allowlist_path: std::path::PathBuf,
        verbs: super::Verbs,
    ) -> Self {
        // One `Reader` is one rendering surface: a render thread and its
        // snapshot. Under WebKit it was a web process, the cost ADR 0032 put
        // at thirty for a thirty-message thread, so it is counted from the
        // moment one is built; `Drop` balances it.
        postio_ui::reader::cost::note_surface_created();

        // The body is drawn by the reading renderer (spec 006): one render
        // thread per reader, no web process, no script. It scrolls in a
        // window of its own; the header and notices above it stay put.
        let view = crate::body_view::BodyView::new(crate::body_view::DEFAULT_RENDER_DEADLINE);
        view.add_css_class("postio-reader-view");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hexpand(true)
            .build();
        let place = Rc::new(Place::new(Rc::clone(&source)));
        let find = Rc::new(crate::body_view::find::FindBar::new(&view));
        let zoom_indicator = Rc::new(crate::body_view::zoom::ZoomIndicator::new(&view));
        zoom_indicator.widget().set_halign(gtk::Align::End);
        zoom_indicator.widget().set_valign(gtk::Align::Start);
        zoom_indicator
            .widget()
            .set_margin_top(crate::widgets::space::S2);
        zoom_indicator
            .widget()
            .set_margin_end(crate::widgets::space::S4);
        let body = gtk::Overlay::builder().child(&scroller).build();
        body.add_overlay(zoom_indicator.widget());

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
        let actions = super::actions::new_for(verbs.received);
        // One bar per verb set `ReaderAction::for_send_state` can return.
        // Exactly one is visible, and for `Sending` none is: cancelling is
        // refused once the submission started and retrying would risk a
        // second copy, so the bar offers nothing rather than a refusal.
        let queued_actions = super::actions::new_for(verbs.queued);
        let stopped_actions = super::actions::new_for(verbs.stopped);

        let chips = super::chips::Chips::new();

        // The header sits above the banner and does not scroll away with
        // the body (#319): it is a sibling in this native box, never markup
        // inside the body's document. The action bar (#498) used to sit
        // last, under the attachment chips; it is in the header now, with
        // the subject, which is where the conversation pane has always put
        // it (#1435).
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&header.widget());
        let under_header = gtk::Box::new(gtk::Orientation::Vertical, 0);
        under_header.add_css_class("postio-reader-under-header");
        under_header.set_visible(false);
        container.append(&under_header);
        let notices = Rc::new(NoticeSlot::new([
            decode_notice.widget(),
            reader_notice.widget(),
            banner.widget(),
            unsubscribe_banner.widget(),
        ]));
        container.append(&notices.widget());
        container.append(find.widget());
        container.append(&body);
        container.append(&chips.widget());
        // **Not appended last any more.** #498 put the bar under the chips,
        // "matching the canvas' footer treatment"; the conversation pane
        // puts the same bar in its header, so the same message drew Reply in
        // two different places depending on which surface opened it -- and
        // for a one-message row that surface is this one, so the older
        // placement was what most mail showed (#1435).
        // A surface that draws its own verbs gets no row of empty bars.
        if !verbs.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.append(&actions.widget());
            row.append(&queued_actions.widget());
            row.append(&stopped_actions.widget());
            header.set_verbs(row.upcast_ref::<gtk::Widget>());
        }

        let reader = Reader {
            container,
            under_header,
            view,
            scroller,
            body,
            find,
            zoom_indicator,
            header,
            banner,
            reader_notice,
            unsubscribe_banner,
            notices,
            unsubscribe_list: Rc::new(RefCell::new(None)),
            on_unsubscribe: Rc::new(RefCell::new(Vec::new())),
            actions,
            queued_actions,
            stopped_actions,
            send_state: Rc::new(std::cell::Cell::new(None)),
            showing: Rc::new(std::cell::Cell::new(false)),
            allowlist,
            thread: Rc::new(RefCell::new(Vec::new())),
            originals: Rc::new(RefCell::new(std::collections::HashMap::new())),
            renders: Rc::new(RefCell::new(
                postio_ui::reader::document::RenderCache::default(),
            )),
            on_message_action: Rc::new(RefCell::new(Vec::new())),
            on_current_message: Rc::new(RefCell::new(Vec::new())),
            open: Rc::new(RefCell::new(None)),
            absent: Rc::new(std::cell::Cell::new(None)),
            highlight: Rc::new(RefCell::new(Vec::new())),
            chips,
            rendered: Rc::new(RefCell::new(Vec::new())),
            on_parts_requested: Rc::new(RefCell::new(Vec::new())),
            paints: Rc::new(std::cell::Cell::new(0)),
            loads: Rc::new(std::cell::Cell::new(0)),
            document: Rc::new(RefCell::new(String::new())),
            place,
            allowlist_path: Rc::new(allowlist_path.clone()),
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
            {
                // The `Show` verb inside a blocked-images notice. Intercepted
                // here because `handle_decide_policy` hands every navigation that
                // leaves the pane to the system browser, and a consent verb must
                // be told apart from a link the sender wrote before that happens.
                let allowlist = Rc::clone(&reader.allowlist);
                let originals = Rc::clone(&reader.originals);
                let renders = Rc::clone(&reader.renders);
                let thread = Rc::clone(&reader.thread);
                let document = Rc::clone(&reader.document);
                let loads = Rc::clone(&reader.loads);
                let place = Rc::clone(&reader.place);
                let notices = Rc::clone(&reader.notices);
                let allowlist_path = allowlist_path.clone();
                let on_message_action = Rc::clone(&reader.on_message_action);
                view.connect_message_verb(move |view, scope, verb| {
                    let verb = match verb {
                        "reply" => Some(MessageVerb::Reply),
                        "forward" => Some(MessageVerb::Forward),
                        "continue" => Some(MessageVerb::Continue),
                        _ => None,
                    };
                    if let Some(verb) = verb {
                        // **No re-render.** Replying opens a composer; redrawing
                        // the document to do it would throw away the scroll
                        // position and every fold the reader had opened.
                        for handler in on_message_action.borrow().iter() {
                            handler(scope, verb);
                        }
                        return;
                    }
                    // `allow`: whose consent this is. The scope names a message
                    // and the message names a sender: allowing "this thread"
                    // would be a different, worse promise than the banner's.
                    let sender = thread
                        .borrow()
                        .iter()
                        .find(|message| message.scope == scope)
                        .map(|message| message.address.clone());
                    if let Some(sender) = sender {
                        let mut list = allowlist.borrow_mut();
                        list.allow(&sender);
                        if let Err(error) = list.save_to(&allowlist_path) {
                            glib::g_warning!(
                                "postio",
                                "could not save the remote-image allow list: {error}"
                            );
                        }
                    }
                    let messages = thread.borrow().clone();
                    place.keep.set(true);
                    place.consented.set(true);
                    load_document(
                        &Canvas {
                            view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
                        },
                        &compose_thread_document(
                            &messages,
                            &allowlist,
                            &originals.borrow(),
                            &renders,
                        ),
                    );
                });
            }
            {
                // Which message has most of the view, as the view scrolls:
                // the rail's mark. Only a scope this document rendered is
                // passed on.
                let on_current_message = Rc::clone(&reader.on_current_message);
                let thread = Rc::clone(&reader.thread);
                view.connect_current_message(move |_, scope| {
                    // Only a scope this document actually rendered.
                    if !thread.borrow().iter().any(|message| message.scope == scope) {
                        return;
                    }
                    for handler in on_current_message.borrow().iter() {
                        handler(scope);
                    }
                });
            }

            {
                // The fallback notice's "View source" (spec 006 FR-023,
                // T105): what was sent, as text, in this pane. Local, and
                // one navigation from the message again.
                let open = Rc::clone(&reader.open);
                let thread = Rc::clone(&reader.thread);
                let document = Rc::clone(&reader.document);
                let loads = Rc::clone(&reader.loads);
                let place = Rc::clone(&reader.place);
                let notices = Rc::clone(&reader.notices);
                view.connect_view_source(move |view| {
                    let source = source_document(open.borrow().as_ref(), &thread.borrow());
                    load_document(
                        &Canvas {
                            view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
                        },
                        &source,
                    );
                });
            }

            let open = Rc::clone(&reader.open);
            let allowlist = Rc::clone(&reader.allowlist);
            // Weakly, and this is the half that is easy to get wrong: the
            // banner's own closures hold the notice (below), so a strong
            // reference back would be a cycle between two Rcs that nothing
            // ever frees -- and both of them hold the body view, so what
            // leaks is a render thread and a snapshot per message.
            // `gtk_reader_teardown` is what says so.
            let banner_from_notice = Rc::downgrade(&reader.banner);
            let highlight = Rc::clone(&reader.highlight);
            let rendered = Rc::clone(&reader.rendered);
            let loads = Rc::clone(&reader.loads);
            let place = Rc::clone(&reader.place);
            let notices = Rc::clone(&reader.notices);
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
                place.keep.set(true);
                render_open(
                    &Canvas {
                        view: &view,
                        document: &document,
                        loads: &loads,
                        place: &place,
                        notices: &notices,
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
            let loads = Rc::clone(&reader.loads);
            let place = Rc::clone(&reader.place);
            let notices = Rc::clone(&reader.notices);
            let document = Rc::clone(&reader.document);
            let banner_weak = banner_weak.clone();
            reader.banner.connect_show_once(move || {
                let Some(reader_notice) = notice_weak.upgrade() else {
                    return;
                };
                if let Some(banner) = banner_weak.upgrade() {
                    place.keep.set(true);
                    place.consented.set(true);
                    render_open(
                        &Canvas {
                            view: &view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
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
            let loads = Rc::clone(&reader.loads);
            let place = Rc::clone(&reader.place);
            let notices = Rc::clone(&reader.notices);
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
                    place.keep.set(true);
                    place.consented.set(true);
                    render_open(
                        &Canvas {
                            view: &view,
                            document: &document,
                            loads: &loads,
                            place: &place,
                            notices: &notices,
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

        reader.reset();
        reader
    }

    /// Get ready for the first message before anything needs it.
    ///
    /// What a first render would wait on is font discovery -- reading every
    /// installed face's tables -- which happens once per process. This
    /// starts it off the main thread, so the first message a person opens
    /// is rendered in full rather than falling back at the deadline. (Under
    /// WebKit this started the reader's web process, #1216.)
    pub fn warm(&self) {
        crate::body_view::prewarm_fonts();
    }

    /// The widget to place in a surface -- the classic app's shell puts it in
    /// `postio_gtk::shell::Shell::reader` -- the header,
    /// notices and body, stacked.
    pub fn widget(&self) -> gtk::Widget {
        self.container.clone().upcast()
    }

    /// The body view -- test-facing, e.g. to watch `rendered` for whether a
    /// snapshot has reached the screen yet.
    pub fn view(&self) -> &crate::body_view::BodyView {
        &self.view
    }

    /// Show or hide the notice slot altogether.
    ///
    /// For a reader whose document says these things itself -- the
    /// conversation's, where each message carries its own blocked-images
    /// verb and the slot would be a bar of empty space above the thread.
    pub fn set_notices_visible(&self, visible: bool) {
        self.notices.widget().set_visible(visible);
    }

    /// Which remote-image policy this reader draws `sender`'s mail under.
    ///
    /// What a caller preparing a body off the main thread has to prepare it
    /// for: the allow list lives here, and a body prepared under the wrong
    /// policy is simply drawn again when shown.
    pub fn remote_images_for(&self, sender: Option<&str>) -> RemoteImages {
        if sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(sender)) {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        }
    }

    /// The allow list as it stands, for a worker that has to decide a
    /// policy before it knows whose message it is reading.
    pub fn allowlist_snapshot(&self) -> RemoteImageAllowList {
        self.allowlist.borrow().clone()
    }

    /// Whether the remote-image banner is currently shown.
    pub fn banner_visible(&self) -> bool {
        self.notices.shows(Notice::RemoteImages)
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
    /// row (`postio_gtk::conversation::ConversationView::build_entry`) — the same
    /// reason [`Reader::header`]'s identity fields get hidden there. The
    /// surface around this reader already carries Reply/Reply all/Forward;
    /// drawing this reader's own copy on top is a duplicate, not a second
    /// opinion.
    pub fn set_actions_visible(&self, visible: bool) {
        self.actions_suppressed.set(!visible);
        if visible {
            self.set_send_state(self.send_state.get());
        } else {
            self.actions.set_visible(false);
            self.queued_actions.set_visible(false);
            self.stopped_actions.set_visible(false);
        }
    }

    /// Say what the message on screen is doing, so the bar offers verbs that
    /// apply to it (#1525, spec 003).
    ///
    /// The reading pane has always assumed a message *arrived* — Reply,
    /// Reply all, Forward and Archive are all answers to somebody else's
    /// mail. Until the Outbox there was no folder holding one that had not
    /// arrived, so the assumption was never wrong; now it is, and on the
    /// message a person is most likely to want to act on.
    ///
    /// Which verbs each state gets is
    /// [`ReaderAction::for_send_state`](postio_ui::reader::header::ReaderAction::for_send_state),
    /// in `postio-ui`, so the macOS reader
    /// reaches the same answer rather than a second copy of this judgement.
    ///
    /// Call it after [`render`](Self::render), which clears it — the same
    /// convention as [`set_unsubscribe`](Self::set_unsubscribe) and
    /// [`set_encoding_problems`](Self::set_encoding_problems), and for the
    /// same reason: this belongs to one message and must not outlive it.
    pub fn set_send_state(&self, state: Option<postio_model::DraftState>) {
        use postio_ui::reader::header::ReaderAction;

        self.send_state.set(state);
        let wanted = ReaderAction::for_send_state(state);
        let is = |verb: ReaderAction| wanted.contains(&verb);

        let show = self.showing.get() && !self.actions_suppressed.get();
        self.actions.set_visible(show && is(ReaderAction::Reply));
        self.queued_actions
            .set_visible(show && is(ReaderAction::CancelSend));
        self.stopped_actions
            .set_visible(show && is(ReaderAction::RetrySend));

        // And the banner, which offers to unsubscribe from the sender's
        // domain when there is no `List-Id` (#971) — the sender of an
        // outgoing message being the user themselves.
        if !ReaderAction::unsubscribable(state) {
            self.unsubscribe_banner.set_list(None);
            self.notices.want(Notice::Unsubscribe, false);
        }
    }

    /// Show the action bar unless [`Reader::set_actions_visible`]`(false)`
    /// has suppressed it — what every call site that used to say
    /// `self.actions.set_visible(true)` means now.
    fn show_actions_unless_suppressed(&self) {
        self.showing.set(true);
        if self.actions_suppressed.get() {
            return;
        }
        // Through `set_send_state` rather than straight to `self.actions`, so
        // a repaint of a message being sent does not put Reply back on it.
        self.set_send_state(self.send_state.get());
    }

    /// Press a verb wherever it is currently drawn. Test-facing.
    ///
    /// Across all three bars on purpose: which one holds a verb depends on
    /// the send state, and a test that reached into one of them by name
    /// could not have caught two bars going unconnected.
    #[doc(hidden)]
    pub fn test_press(&self, command: postio_core::CommandId) {
        for bar in [&self.actions, &self.queued_actions, &self.stopped_actions] {
            if bar.button(command).is_some() {
                bar.press(command);
                return;
            }
        }
    }

    /// The action bar's widget, so a test can ask where it is mounted.
    #[doc(hidden)]
    pub fn actions_widget(&self) -> gtk::Widget {
        self.actions.widget()
    }

    /// Whether the action bar is currently on screen. For tests.
    #[doc(hidden)]
    pub fn actions_visible(&self) -> bool {
        self.actions.widget().is_visible()
    }

    /// The verbs a person can actually see and press, by their labels.
    ///
    /// Across all three bars, because which one is showing is the thing
    /// under test: asking `actions_visible` alone cannot tell "Reply is
    /// offered" from "Send again is offered" (#1525).
    #[doc(hidden)]
    pub fn visible_verbs(&self) -> Vec<String> {
        let mut found = Vec::new();
        for bar in [&self.actions, &self.queued_actions, &self.stopped_actions] {
            let widget = bar.widget();
            if !widget.is_visible() {
                continue;
            }
            collect_labels(&widget, &mut found);
        }
        found
    }

    /// The banner's "always allow" button label, naming whichever sender it
    /// would exempt.
    pub fn banner_always_allow_label(&self) -> String {
        self.banner.always_allow_label()
    }

    /// Run one of the banners' commands, as its button would: `show_images`,
    /// `always_show_images` or `unsubscribe`. Only when the message has what
    /// the banner offers -- images held back, a list to leave -- whichever
    /// notice the slot happens to be showing; a key on a message with nothing
    /// to show or no list to leave does nothing. Returns whether it ran.
    ///
    /// The registry entries these answer were buttons and nothing else, so a
    /// person without a pointer could not reach them (Principle II;
    /// `specs/005-tui-frontend` T044, T048).
    pub fn run_banner_command(&self, command: postio_core::CommandId) -> bool {
        use postio_core::CommandId;
        match command {
            CommandId::ShowImages if self.notices.wanted(Notice::RemoteImages) => {
                self.banner.emit_show_once()
            }
            CommandId::AlwaysShowImages if self.notices.wanted(Notice::RemoteImages) => {
                self.banner.emit_always_allow()
            }
            CommandId::Unsubscribe if self.notices.wanted(Notice::Unsubscribe) => {
                self.unsubscribe_banner.emit_unsubscribe()
            }
            _ => return false,
        }
        true
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
        self.notices.shows(Notice::Unsubscribe)
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

    /// Take messages rendered ahead of being shown, so that drawing them
    /// parses nothing on this thread. See
    /// [`postio_ui::reader::document::RenderCache::offer`].
    pub fn offer_prepared(&self, prepared: Vec<postio_ui::reader::document::Prepared>) {
        self.renders.borrow_mut().offer(prepared);
    }

    /// Render `body` into the pane.
    ///
    /// `sender` is the allow-list key: with a sender already on the standing
    /// allow list, remote images load without the banner appearing at all.
    /// Otherwise they stay blocked until the banner's "show once" or "always
    /// allow" is used — both re-render through this same [`Open`] state, so
    /// a caller never has to.
    pub fn render(&self, body: &MessageBody, sender: Option<&str>) {
        self.render_prepared(body, sender, None);
    }

    /// [`render`](Self::render), with the two parses a message costs
    /// already done elsewhere.
    ///
    /// `prepared` is `postio_ui::reader::document::prepare_message` run on a
    /// worker, under the policy [`remote_images_for`](Self::remote_images_for)
    /// gives this sender. Whether `body` is bulk mail and what the sanitiser
    /// makes of it are both html5ever over the whole body, and both were paid
    /// on the main thread for every message the cursor settled on. With it,
    /// this only hands the view a document; without it, or when it was
    /// prepared for something else, it draws exactly as `render` always has.
    pub fn render_prepared(
        &self,
        body: &MessageBody,
        sender: Option<&str>,
        prepared: Option<postio_ui::reader::document::Prepared>,
    ) {
        self.paints.set(self.paints.get() + 1);
        self.absent.set(None);
        // Cleared here rather than left to the caller. A caveat that outlived
        // the message it was about would be worse than never showing one --
        // it would put "these may not be the sender's words" over mail that
        // decoded perfectly, and a warning that is sometimes wrong is one
        // people learn to ignore. Callers turn it back on for the message
        // they are showing, through `set_encoding_problems`.
        self.notices.want(Notice::Decode, false);
        self.set_unsubscribe(None);
        // Per-message, like the two above: a message drawn over one that
        // was being sent must not inherit its bar.
        self.set_send_state(None);
        // Every message opens as its sender built it (spec 006 FR-031).
        // Whether it reads as bulk is still asked: it picks the sheet the
        // original is drawn on.
        // The message on screen drawn again keeps its place; another
        // message starts at the top. This pane has no message identity, so
        // "the same" is the same body from the same sender: two senders'
        // identical notifications are two messages.
        let same = self
            .open
            .borrow()
            .as_ref()
            .is_some_and(|open| open.body == *body && open.sender.as_deref() == sender);
        self.place.keep.set(same);
        // What a render past its deadline shows instead (spec 006 FR-023):
        // the message's own text part. A message with none keeps the notice
        // and its "View source" alone -- deriving text from the HTML would
        // put an html5ever parse back on this thread, which
        // `render_prepared` exists to keep off it.
        self.place
            .plain
            .replace(body.text.clone().unwrap_or_default());
        let bulk = prepared
            .as_ref()
            .and_then(|prepared| prepared.verdict_for(body))
            .unwrap_or_else(|| postio_ui::reader::document::suits_reader_view(body));
        let rendering = postio_ui::reader::document::opening_rendering();
        // What the person always wants for this sender is the choice the
        // message opens with; `O` overrules it for this message alone.
        let remembered = sender.and_then(|sender| self.allowlist.borrow().treatment_for(sender));
        *self.open.borrow_mut() = Some(Open {
            body: body.clone(),
            sender: sender.map(str::to_owned),
            rendering,
            bulk,
            prepared,
            chosen: remembered,
            remembered,
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

    fn compose_thread(&self, messages: &[ThreadMessage]) -> String {
        compose_thread_document(
            messages,
            &self.allowlist,
            &self.originals.borrow(),
            &self.renders,
        )
    }

    /// Whether [`render_thread`](Self::render_thread) would change anything.
    ///
    /// Composing a document is cheap; handing it to the view is not -- it is
    /// a full style, layout and paint on the render thread. So a caller that
    /// cannot easily tell whether its redraw is needed can ask.
    pub fn would_render_thread(&self, messages: &[ThreadMessage]) -> bool {
        self.compose_thread(messages) != *self.document.borrow()
    }

    /// Draw `messages` if the document they make differs from the one on
    /// screen, and say whether it did.
    ///
    /// One compose, where the pair `would_render_thread` then `render_thread`
    /// was two (#1605): the answer to "would it change" is the document, and
    /// the document is what gets loaded.
    pub fn render_thread_if_changed(&self, messages: &[ThreadMessage]) -> bool {
        let document = self.compose_thread(messages);
        if document == *self.document.borrow() {
            return false;
        }
        self.load_thread(messages, &document);
        true
    }

    /// Draw a whole conversation into this one view (ADR 0032, #1316).
    ///
    /// The experiment behind #1316: one thread is one document is one view,
    /// whatever the thread's length. Under WebKit the stacked pane's reader
    /// per expanded message was a web process each, so a thirty-message
    /// thread ended with thirty of them.
    ///
    /// # Remote images stay blocked here, deliberately
    ///
    /// The allow list is a decision about *a sender*, and a document is one
    /// document for all of it. Allowing one sender's images in
    /// a thread would allow every sender's in that thread, which is not what
    /// anybody agreed to.
    ///
    /// That is no longer what happens. #1353 made the decision per message,
    /// from its own sender's place in the allowlist, so an allowed
    /// correspondent no longer carries the rest of the thread with them — and
    /// #1398 does the same for reader view, which each message decides for
    /// itself and `⌃O` overrules one at a time. This comment said the whole
    /// document was `Blocked` long after it had stopped being true.
    pub fn render_thread(&self, messages: &[ThreadMessage]) {
        let document = self.compose_thread(messages);
        self.load_thread(messages, &document);
    }

    /// Load a composed thread document, and reset what a new document resets.
    fn load_thread(&self, messages: &[ThreadMessage], document: &str) {
        // The same conversation again -- a late body, a message shown whole,
        // a flag changing -- keeps the reader's place in it. Any message in
        // common is enough: a thread that grew is still the one they were
        // reading.
        let same = self
            .thread
            .borrow()
            .iter()
            .any(|drawn| messages.iter().any(|message| message.scope == drawn.scope));
        self.place.keep.set(same);
        self.place.plain.replace(thread_plain_text(messages));
        self.thread.replace(messages.to_vec());
        self.paints.set(self.paints.get() + 1);
        self.absent.set(None);
        self.set_unsubscribe(None);
        // Per-message, like the two above: a message drawn over one that
        // was being sent must not inherit its bar.
        self.set_send_state(None);
        self.notices.clear();
        // A conversation is drawn as the classic reader draws it, with no
        // treatment to name.
        self.place.treated.set(None);
        show_treatment(&self.place, None);
        load_document(&self.canvas(), document);
    }

    /// Draw one message of a thread as its sender wrote it — `⌃O`.
    ///
    /// Per message, which is what the single-message [`Reader::view_original`] has
    /// always promised and what a pane holding several has to mean: showing
    /// one newsletter whole says nothing about the message below it.
    ///
    /// A no-op when no thread is on screen, so the key is safe to press
    /// anywhere, and when that message is already whole.
    pub fn view_original_for(&self, scope: &str) {
        if self.thread.borrow().is_empty() {
            return;
        }
        if self
            .originals
            .borrow_mut()
            .insert(scope.to_owned(), Rendering::Original)
            == Some(Rendering::Original)
        {
            return;
        }
        let thread = self.thread.borrow().clone();
        self.render_thread(&thread);
    }

    /// Draw the message `scope` in reader view, or as its sender built it
    /// again if it already is — the one-document pane's `toggle_reader_view`
    /// (spec 006 FR-031). Per message: the rest of the thread keeps however
    /// it was drawn.
    pub fn toggle_reader_view_for(&self, scope: &str) {
        if self.thread.borrow().is_empty() {
            return;
        }
        {
            let mut chosen = self.originals.borrow_mut();
            let now = chosen
                .get(scope)
                .copied()
                .unwrap_or_else(postio_ui::reader::document::opening_rendering);
            let next = if now == Rendering::Reader {
                Rendering::Original
            } else {
                Rendering::Reader
            };
            chosen.insert(scope.to_owned(), next);
        }
        let thread = self.thread.borrow().clone();
        self.render_thread(&thread);
    }

    /// Draw the open message in reader view, or as its sender built it again
    /// — the single-message reader's `toggle_reader_view` (spec 006 FR-031).
    ///
    /// A no-op for a message with no markup of its own: plain text is already
    /// what reader view is trying to get back to.
    pub fn toggle_reader_view(&self) {
        {
            let mut guard = self.open.borrow_mut();
            let Some(open) = guard.as_mut() else { return };
            if open
                .body
                .html
                .as_deref()
                .is_none_or(|html| html.trim().is_empty())
            {
                return;
            }
            open.rendering = if open.rendering == Rendering::Reader {
                Rendering::Original
            } else {
                Rendering::Reader
            };
        }
        self.rerender();
    }

    /// Forget which messages were asked for whole.
    ///
    /// Called when the conversation changes, not on every redraw: a body
    /// arriving re-renders the thread, and clearing there would undo the
    /// choice the moment the rest of the thread loaded.
    pub fn forget_originals(&self) {
        self.originals.borrow_mut().clear();
    }

    /// Draw the message in its owner's column instead of a pane of its own
    /// (Focus's open message, screen 04): the header, the notices, the body
    /// and the parts scroll together in `scroller`, and the body is drawn
    /// flat on the column's ground -- no frame around correspondence, at the
    /// column's measure -- keeping a quiet one only for mail that paints its
    /// own page. What the view rasterises is still only what `scroller`
    /// shows ([`BodyView::flow_in`](crate::body_view::BodyView::flow_in)).
    ///
    /// The attachments are drawn as cards, not pills, and the notice slot
    /// takes no room while it has nothing to say. Called once, before
    /// anything is shown.
    ///
    /// The find bar leaves the column: a bar at the column's top is only on
    /// screen at the top, so opening it would scroll the message there. The
    /// owner places [`find_bar`](Self::find_bar) above `scroller` instead.
    ///
    /// The body's ground is the column's, read from a probe that wears
    /// `.postio-flow-ground` -- whose `color` the owner's stylesheet sets to
    /// the column's ground token -- each time the body is drawn.
    pub fn flow_in(&self, scroller: &gtk::ScrolledWindow) {
        self.scroller.set_child(None::<&gtk::Widget>);
        self.body.set_child(Some(&self.view));
        self.view.flow_in(scroller);
        self.place.flow.set(true);
        self.chips.set_cards(true);
        self.notices.set_collapsing(true);
        if let Some(container) = self.find.widget().parent().and_downcast::<gtk::Box>() {
            container.remove(self.find.widget());
            let ground = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            ground.add_css_class("postio-flow-ground");
            ground.set_accessible_role(gtk::AccessibleRole::Presentation);
            container.append(&ground);
            self.view.set_ground(&ground);
            // The rest of the column's palette, the same way: a body in the
            // app's colours is drawn in the column's own tokens (T211).
            for (variable, class) in FLOW_PALETTE {
                let probe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                probe.add_css_class(class);
                probe.set_accessible_role(gtk::AccessibleRole::Presentation);
                container.append(&probe);
                self.view.add_palette_probe(variable, &probe);
            }
        }
    }

    /// Draw every single message under a treatment (specs/007-postio-focus
    /// T210-T213): app colours or the original on paper, as the rule in
    /// `postio_body::treatment` decides or the person chose, with a quiet
    /// line above an HTML body naming which and offering the other.
    ///
    /// Focus's open message calls this once, before anything is shown; the
    /// classic reader never does, and draws as it always has. A conversation
    /// drawn as one document ([`render_thread`](Self::render_thread)) is not
    /// treated.
    pub fn use_treatments(&self) {
        if self.place.treatments.replace(true) {
            return;
        }
        let line = RenderModeLine::new();
        line.set_key(
            postio_core::Keymap::resolve(&Default::default())
                .binding(postio_core::CommandId::SwitchTreatment),
        );
        let switcher = self.switcher();
        line.connect_switch({
            let switcher = switcher.clone();
            move || {
                switcher.switch();
            }
        });
        line.connect_always(move || {
            switcher.remember();
        });
        // Directly over the body, under any notice: the line is about the
        // body, and a notice above it is about the message (T208's
        // "render-mode line -> body 12").
        self.container
            .insert_child_after(&line.widget(), Some(&self.notices.widget()));
        self.place.line.replace(Some(line));
    }

    /// The treatment the body on screen is drawn in: what the layout around
    /// it reads to size its column (T207). App colours when nothing is
    /// drawn under one -- plain text, a conversation, an empty pane, or a
    /// reader that does not draw treatments.
    pub fn treatment(&self) -> Treatment {
        self.place
            .treated
            .get()
            .map(|treated| treated.shown)
            .unwrap_or_default()
    }

    /// How the body on screen was treated, and what the rule had chosen;
    /// `None` when nothing is drawn under a treatment.
    pub fn treated(&self) -> Option<Treated> {
        self.place.treated.get()
    }

    /// Called with the treatment each time a body is drawn under one: a
    /// message opening, `O`, a remembered choice. The same treatment can
    /// arrive twice in a row; a caller sizing a column to it is idempotent.
    pub fn connect_treatment_changed(&self, handler: impl Fn(Treatment) + 'static) {
        self.place.on_treatment.borrow_mut().push(Rc::new(handler));
    }

    /// Draw the open message in the other treatment -- `O`
    /// (`switch_treatment`). For this message only: the next one opens as
    /// the rule or its sender's remembered choice says. False, and nothing
    /// drawn, when there is no HTML body under a treatment to switch.
    pub fn switch_treatment(&self) -> bool {
        self.switcher().switch().is_some()
    }

    /// Remember the treatment on screen for the open message's sender --
    /// "Always for this sender" -- beside their remote-image setting. False
    /// when there is no sender or no treatment to remember.
    pub fn remember_treatment(&self) -> bool {
        self.switcher().remember().is_some()
    }

    /// The render-mode line, once [`use_treatments`](Self::use_treatments)
    /// made one. Test-facing.
    #[doc(hidden)]
    pub fn render_mode_line(&self) -> Option<Rc<RenderModeLine>> {
        self.place.line.borrow().clone()
    }

    /// The scale a paper body was zoomed by to fit the column, on top of
    /// the person's own zoom ([`zoom`](Self::zoom)): 1.0 when it fitted or
    /// is not on paper, and never under `postio_render::render::PAPER_FIT_FLOOR`,
    /// below which the sheet scrolls sideways instead (T207, T212).
    ///
    /// Read off the snapshot on screen, so it answers for what is drawn: the
    /// renderer fits a paper body to whatever width the view is given, and
    /// a column that changes width is drawn again and fitted again.
    pub fn paper_fit(&self) -> f64 {
        self.view.document().map_or(1.0, |document| document.fit)
    }

    fn switcher(&self) -> Switcher {
        Switcher {
            view: self.view.downgrade(),
            document: Rc::clone(&self.document),
            loads: Rc::clone(&self.loads),
            place: Rc::downgrade(&self.place),
            notices: Rc::clone(&self.notices),
            banner: Rc::downgrade(&self.banner),
            reader_notice: Rc::downgrade(&self.reader_notice),
            open: Rc::clone(&self.open),
            highlight: Rc::clone(&self.highlight),
            rendered: Rc::clone(&self.rendered),
            allowlist: Rc::clone(&self.allowlist),
            allowlist_path: Rc::clone(&self.allowlist_path),
        }
    }

    /// How far down the pane is scrolled, in view pixels. Test-facing.
    ///
    /// Without an observable #1431 was invisible: the three scrolling
    /// methods returned having done nothing and every caller reported
    /// success.
    #[doc(hidden)]
    pub fn scrolled_for_test(&self) -> f64 {
        self.view.scrolled()
    }

    /// The document currently composed for the open thread. Test-facing.
    #[doc(hidden)]
    pub fn document_for_test(&self) -> String {
        self.compose_thread(&self.thread.borrow().clone())
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
        self.notices.shows(Notice::ReaderView)
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

    /// The view and its load bookkeeping, together — see [`Canvas`].
    fn canvas(&self) -> Canvas<'_> {
        Canvas {
            view: &self.view,
            document: &self.document,
            loads: &self.loads,
            place: &self.place,
            notices: &self.notices,
        }
    }

    /// Called every time a render settles how many remote references are
    /// being held back — see [`RenderedHandler`].
    ///
    /// Fires on the initial render and again whenever the banner's "show
    /// once" or "always allow" changes the count, so a caller wiring the
    /// parts panel's `postio_gtk::parts::PartsPanel::set_held_back` never goes
    /// stale.
    pub fn connect_rendered(&self, handler: impl Fn(HeldBack) + 'static) {
        self.rendered.borrow_mut().push(Box::new(handler));
    }

    /// Put `widget` under the message header and over the notices and the
    /// body -- Focus's marker card -- or take away what is there.
    pub fn set_under_header(&self, widget: Option<&gtk::Widget>) {
        while let Some(child) = self.under_header.first_child() {
            self.under_header.remove(&child);
        }
        if let Some(widget) = widget {
            self.under_header.append(widget);
        }
        self.under_header.set_visible(widget.is_some());
    }

    /// Draw the message's attachments as chips under the body.
    ///
    /// Metadata only, and deliberately: a chip is drawn from what
    /// `BODYSTRUCTURE` already said, so a message nothing has been fetched
    /// for still shows what came with it. See [`super::chips`].
    pub fn set_attachments(&self, root: &str, parts: &[postio_model::Attachment]) {
        self.chips.set_parts(root, parts);
    }

    /// Called when one of those chips is activated.
    ///
    /// The chip does not act — it asks. Whoever wires this opens
    /// the classic app's parts panel, which is where the verbs live.
    pub fn connect_attachment(&self, handler: impl Fn(&postio_ui::reader::parts::Node) + 'static) {
        self.chips.connect_activated(handler);
    }

    /// Ask for the parts panel — `p`, the keyboard's way in when there is no
    /// chip to click. Same destination as [`Reader::connect_attachment`],
    /// with no particular part in hand: it opens on whatever the pane is
    /// showing, same as clicking any chip does today.
    /// Called when a message's own reply or forward is activated, with the
    /// scope that message was rendered under.
    ///
    /// A scope rather than a `MessageId` because that is what the document
    /// carries; the conversation view owns the mapping back, as it already
    /// does for the per-message bars in the stacked pane.
    /// Called with the scope of the message filling most of the pane, as the
    /// reader scrolls.
    ///
    /// The rail's own rule decides *which* that is —
    /// [`postio_ui::reader::rail::current`] — from extents the observer
    /// measures. What arrives here is already the answer.
    pub fn connect_current_message(&self, handler: impl Fn(&str) + 'static) {
        self.on_current_message.borrow_mut().push(Box::new(handler));
    }

    /// Called with a message's scope and verb when its own reply, forward
    /// or continue is activated in the document.
    pub fn connect_message_action(&self, handler: impl Fn(&str, MessageVerb) + 'static) {
        self.on_message_action.borrow_mut().push(Box::new(handler));
    }

    /// Called when `p` asks for the parts panel.
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
    /// reaches the keyboard's. See the classic `Window::apply_keymap` for where this is
    /// called from, alongside the finder, the cheat sheet and the parts
    /// panel's own copies.
    ///
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        // All three, for the reason `connect_command` gives: a bar nobody
        // hands a keymap to draws a verb with no key on it, and these two are
        // the ones a person meets when a send has gone wrong.
        self.actions.set_keymap(keymap);
        self.queued_actions.set_keymap(keymap);
        self.stopped_actions.set_keymap(keymap);
        // The notice's own cap, from the same keymap. Written down here it
        // would go on saying `C-o` after a rebind moved the key, which is the
        // drift `KeycapButton` exists to end (#1002).
        self.reader_notice
            .set_action_key(keymap.binding(postio_core::CommandId::ViewOriginal));
        if let Some(line) = self.place.line.borrow().as_ref() {
            line.set_key(keymap.binding(postio_core::CommandId::SwitchTreatment));
        }
    }

    /// Called with the invocation whenever a button in the action bar is
    /// pressed — the same [`postio_core::Command`] the keyboard's binding for
    /// the same verb would produce. See
    /// `postio_gtk::list_view::MessageListView::connect_command` for the shared shape;
    /// whoever mounts the reader hands this straight to the same
    /// `Window::act` the list's row actions do.
    pub fn connect_command(&self, handler: impl Fn(postio_core::Command) + 'static) {
        // **Every bar, not just the first.** There are three -- one per verb
        // set `ReaderAction::for_send_state` can return -- and only the
        // received-mail one was ever connected. So "Send again" on a failed
        // send and "Cancel send" on a queued one were drawn, were clickable,
        // and did nothing at all: no toast, no rejection, nothing, because
        // the command was never raised for anything to reject.
        //
        // Reported as the button not working, and it looked like a dispatch
        // or resolution bug all the way down -- the session resolves
        // `RetrySend { draft: None }` from the row in view perfectly well.
        // Nothing was ever asking it to.
        let handler = std::rc::Rc::new(handler);
        for bar in [&self.actions, &self.queued_actions, &self.stopped_actions] {
            let handler = handler.clone();
            bar.connect_command(move |command| handler(command));
        }
    }

    /// Fetch remote images with `fetch`: the owner's, because this crate
    /// speaks no protocol. Asked only for a document that names remote
    /// images, which it does only for a sender the user allowed or a
    /// message they chose to show once (spec 006 FR-025, T137).
    pub fn set_remote_fetch(&self, fetch: impl Fn(Vec<String>, RemoteArrived) + 'static) {
        self.place.fetch.replace(Some(Rc::new(fetch)));
    }

    /// Darken the message on screen, or show it as sent again
    /// (`darken_message`, spec 006 FR-013a). False when it does not apply:
    /// not the dark theme, or not a message on paper.
    pub fn darken_message(&self) -> bool {
        self.view.toggle_darken()
    }

    /// `darken_message`'s title for what is on screen, or `None` when the
    /// command does not apply to it.
    pub fn darken_title(&self) -> Option<&'static str> {
        self.view.darken_title()
    }

    /// Open find in the message (`find_in_message`, FR-018).
    ///
    /// The reading position stays: in a column the bar is above it, not at
    /// its top ([`flow_in`](Self::flow_in)), and the first match is the
    /// first one from where the person is reading.
    pub fn find_in_message(&self) {
        self.find.open();
    }

    /// Close find, clearing its highlights.
    pub fn close_find(&self) {
        self.find.close();
    }

    /// The next match (`find_next`), or the previous (`find_previous`).
    pub fn find_step(&self, forward: bool) {
        self.view.find_step(forward);
    }

    /// The find bar: its entry, and where it is placed.
    pub fn find_bar(&self) -> &crate::body_view::find::FindBar {
        &self.find
    }

    /// Whether find is open.
    pub fn finding(&self) -> bool {
        self.find.widget().is_search_mode()
    }

    /// Zoom the message one step in (`zoom_in`, FR-021).
    pub fn zoom_in(&self) {
        self.view.zoom_in();
    }

    /// Zoom the message one step out (`zoom_out`).
    pub fn zoom_out(&self) {
        self.view.zoom_out();
    }

    /// Back to actual size (`zoom_reset`).
    pub fn zoom_reset(&self) {
        self.view.zoom_reset();
    }

    /// The zoom, in percent.
    pub fn zoom(&self) -> u16 {
        self.view.zoom()
    }

    /// Draw at `percent`, snapped to a step: `[reader] zoom` from
    /// `config.toml`.
    pub fn set_zoom(&self, percent: u16) {
        self.view.set_zoom_percent(percent);
    }

    /// What the zoom indicator says, and whether it shows. Test-facing.
    #[doc(hidden)]
    pub fn zoom_indicator(&self) -> Option<String> {
        self.zoom_indicator
            .widget()
            .is_visible()
            .then(|| self.zoom_indicator.label())
    }

    /// Call `f` with the new zoom each time it changes, however it was
    /// changed: a key, Ctrl+scroll, a pinch or the indicator's reset.
    pub fn connect_zoom_changed(&self, f: impl Fn(u16) + 'static) {
        self.view
            .connect_local("zoom-changed", false, move |values| {
                let view = values[0]
                    .get::<crate::body_view::BodyView>()
                    .expect("the signal's own view");
                f(view.zoom());
                None
            });
    }

    /// Paint `terms` wherever they appear in the body.
    ///
    /// What canvas 2b means by "preview · match highlighted": the same
    /// hardened pane, with the reason this message is a hit picked out in it.
    /// Marking happens after sanitizing (see [`postio_ui::search::mark_html`]),
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
    /// It stays inside the same body view rather than becoming an overlay: one widget owns the pane, and the document is built by the
    /// same [`wrap_document`] with remote images blocked, so a state plate
    /// can no more reach the network than a message can.
    pub fn show_absent(&self, state: Absent) {
        self.paints.set(self.paints.get() + 1);
        *self.open.borrow_mut() = None;
        self.absent.set(Some(state));
        // A plate has no text of its own, and must not fall back to the
        // text of the message before it.
        self.place.plain.replace(String::new());
        // Nor a treatment: there is no body to name one for.
        self.place.treated.set(None);
        show_treatment(&self.place, None);
        // A message is still open here — headers arrived, only the body has
        // not — so Reply, Forward and Archive stay reachable exactly as they
        // are from the keyboard while the pane explains why there is no body
        // yet. Only `clear()`'s "nothing selected at all" hides the bar.
        self.show_actions_unless_suppressed();
        // Every notice, not only the images banner: reader view, a decode
        // caveat and the list were all about the message before, and a plate
        // under them put them over a message they said nothing about.
        self.set_unsubscribe(None);
        self.notices.clear();
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

    /// How many documents this pane has actually handed to the view.
    ///
    /// [`paints`](Self::paints) counts times the pane was *asked* to draw;
    /// this counts the times that cost a render. The two differ exactly
    /// where #749 lived: an arrival that recomposes the document
    /// byte-for-byte identically is a paint that must not be a load,
    /// because every load is a whole render on the render thread.
    #[doc(hidden)]
    pub fn loads(&self) -> u32 {
        self.loads.get()
    }

    /// The document the pane last handed to the view.
    ///
    /// What was painted is the view's snapshot ([`Reader::view`]); this is
    /// the finished document before it, the place a composition mistake
    /// shows. Not meant for anything but tests.
    #[doc(hidden)]
    pub fn test_document(&self) -> String {
        self.document.borrow().clone()
    }

    /// Which [`Absent`] the pane is explaining, or `None` if it has a body.
    ///
    /// The seam the wiring tests assert on: proving the *application* stopped
    /// drawing blank panes means asking what the reader was told, which does
    /// not require driving the renderer to a paint.
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
        self.notices.want(Notice::Decode, problems);
    }

    /// Whether the decode caveat is on screen.
    pub fn shows_encoding_problems(&self) -> bool {
        self.notices.shows(Notice::Decode)
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
        self.notices
            .want(Notice::Unsubscribe, list_identifier.is_some());
    }

    /// Called with the list identifier when the unsubscribe banner's button
    /// is activated — the reader only asks; a caller decides what leaving a
    /// list means (`postio-gtk` has no SQL to log the activation itself).
    pub fn connect_unsubscribe_activated(&self, handler: impl Fn(&str) + 'static) {
        self.on_unsubscribe.borrow_mut().push(Box::new(handler));
    }

    /// Empty the pane: no message, no bar, no notice.
    pub fn clear(&self) {
        self.reset();
        load_document(
            &self.canvas(),
            &wrap_document("", RemoteImages::Blocked, Sheet::Theme),
        );
        for handler in self.rendered.borrow().iter() {
            handler(HeldBack::default());
        }
    }

    /// Everything [`clear`](Self::clear) resets, without the empty document.
    ///
    /// What a new reader starts from (#1603): the window's reader is built
    /// before the first frame, so the constructor must not render anything.
    fn reset(&self) {
        *self.open.borrow_mut() = None;
        self.absent.set(None);
        self.place.treated.set(None);
        show_treatment(&self.place, None);
        self.place.plain.replace(String::new());
        self.header.clear();
        // Nothing occupies the pane now, which is what `set_send_state`
        // below reads to decide that no bar belongs on it.
        self.showing.set(false);
        self.actions.set_visible(false);
        self.set_unsubscribe(None);
        self.notices.clear();
        // Per-message, like the two above: a message drawn over one that
        // was being sent must not inherit its bar.
        self.set_send_state(None);
    }

    /// Whether there is anything on screen to scroll.
    ///
    /// **Two fields, because there are two panes.** `open` is set by
    /// [`render`](Self::render) and describes a single message;
    /// `thread` is set by [`render_thread`](Self::render_thread) and
    /// describes a conversation. `render_thread` has never touched `open`.
    ///
    /// The three scrolling methods below all guarded on `open` alone, so
    /// every one of them was a no-op in the one-document pane -- which is the
    /// pane the application now opens conversations in. `scroll_to_message`
    /// said in its own doc comment that it was "a no-op when the pane is not
    /// showing a thread", and did exactly the opposite (#1431).
    ///
    /// It survived #1402's tests because they assert that
    /// `ConversationView::page` *returned true*, which it did: it found a
    /// document reader and called this. Nothing asked whether the page
    /// turned.
    fn showing(&self) -> bool {
        self.open.borrow().is_some() || !self.thread.borrow().is_empty()
    }

    /// Scroll the pane down by about a screenful, without moving the
    /// keyboard off wherever it already is (#438).
    ///
    /// A no-op with nothing open, and at the end of the document -- walking
    /// past the end of a message is a stop, not a wrap-around or an error.
    pub fn page_down(&self) {
        if !self.showing() {
            return;
        }
        self.view.page(true);
    }

    /// Scroll the pane `lines` steps (negative is up). See
    /// [`Reader::page_down`].
    pub fn scroll_lines(&self, lines: i32) {
        if self.showing() {
            self.view.scroll_lines(lines);
        }
    }

    /// Scroll the pane to its top, or to the end of its document.
    pub fn scroll_to_edge(&self, bottom: bool) {
        if self.showing() {
            self.view.scroll_to_edge(bottom);
        }
    }

    /// Scroll a thread document to one of its messages: its top, read from
    /// the snapshot's geometry.
    ///
    /// A no-op when the pane is not showing a thread, so the caller does not
    /// have to ask which pane it is talking to.
    pub fn scroll_to_message(&self, scope: &str) {
        if !self.showing() {
            return;
        }
        self.view.scroll_to_message(scope);
    }

    /// Scroll the pane up by about a screenful. See [`Reader::page_down`].
    pub fn page_up(&self) {
        if !self.showing() {
            return;
        }
        self.view.page(false);
    }
}

/// The view and the bookkeeping that goes with handing it a document.
///
/// Bundled rather than passed as four more arguments because every caller
/// needs all of them together: loading a document is exactly the moment the
/// tally moves, the place is kept or reset, and what is on screen changes.
struct Canvas<'a> {
    view: &'a crate::body_view::BodyView,
    /// The last document handed to the view, kept for
    /// [`Reader::test_document`].
    document: &'a RefCell<String>,
    /// Documents actually handed to the view — [`Reader::loads`].
    loads: &'a Rc<std::cell::Cell<u32>>,
    /// Where the reader is, and whether this load keeps it.
    place: &'a Rc<Place>,
    /// Which notices the drawn message raises.
    notices: &'a NoticeSlot,
}

/// What `O` and the render-mode line's two controls need: the open message
/// and everything [`render_open`] draws it with, and where the allow list is
/// saved. The view and the place are held weakly, because the line's own
/// buttons keep a `Switcher`, the line lives in the reader's widgets, and the
/// view's signal handlers hold the place: a strong hold either way would be
/// a cycle keeping every reader's renderer alive.
#[derive(Clone)]
struct Switcher {
    view: glib::WeakRef<crate::body_view::BodyView>,
    document: Rc<RefCell<String>>,
    loads: Rc<std::cell::Cell<u32>>,
    place: std::rc::Weak<Place>,
    notices: Rc<NoticeSlot>,
    banner: std::rc::Weak<RemoteImageBanner>,
    reader_notice: std::rc::Weak<crate::widgets::NoticeBar>,
    open: Rc<RefCell<Option<Open>>>,
    highlight: Rc<RefCell<Vec<String>>>,
    rendered: Rc<RefCell<Vec<RenderedHandler>>>,
    allowlist: Rc<RefCell<RemoteImageAllowList>>,
    allowlist_path: Rc<std::path::PathBuf>,
}

impl Switcher {
    /// Draw the open message in the other treatment, and say which.
    fn switch(&self) -> Option<Treatment> {
        let place = self.place.upgrade()?;
        let view = self.view.upgrade()?;
        let banner = self.banner.upgrade()?;
        let reader_notice = self.reader_notice.upgrade()?;
        let next = place
            .treated
            .get()
            .filter(|treated| treated.html)?
            .shown
            .other();
        let sender = {
            let mut open = self.open.borrow_mut();
            let current = open.as_mut()?;
            current.chosen = Some(next);
            current.sender.clone()
        };
        let remote = if sender.is_some_and(|sender| self.allowlist.borrow().is_allowed(&sender)) {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        };
        // The same message, drawn again: the reader keeps their place.
        place.keep.set(true);
        render_open(
            &Canvas {
                view: &view,
                document: &self.document,
                loads: &self.loads,
                place: &place,
                notices: &self.notices,
            },
            &banner,
            &reader_notice,
            &self.open,
            &self.highlight,
            remote,
            &self.rendered,
        );
        // An outcome, never content: which treatment, not whose mail.
        tracing::debug!(
            treatment = next.attribute_value(),
            "switched the open message's treatment"
        );
        Some(next)
    }

    /// Remember the treatment on screen for the open message's sender.
    fn remember(&self) -> Option<Treatment> {
        let place = self.place.upgrade()?;
        let shown = place.treated.get().filter(|treated| treated.html)?.shown;
        let sender = {
            let mut open = self.open.borrow_mut();
            let current = open.as_mut()?;
            current.chosen = Some(shown);
            current.remembered = Some(shown);
            current.sender.clone()?
        };
        {
            let mut list = self.allowlist.borrow_mut();
            list.set_treatment(&sender, Some(shown));
            if let Err(error) = list.save_to(&self.allowlist_path) {
                tracing::warn!(%error, "could not save a sender's treatment");
            }
        }
        show_treatment(&place, Some(shown));
        tracing::info!(
            treatment = shown.attribute_value(),
            "remembered a treatment for a sender"
        );
        Some(shown)
    }
}

/// Hand `document` to the view, and count it.
///
/// Every call here is a whole render on the render thread, so a load is a
/// cost, and the tally is the observable a test can hold it to.
/// Deciding whether a load is *needed* is deliberately not done here: this
/// pane is handed a body and a sender, not a message, so it cannot tell a
/// second message that happens to compose an identical document from the same
/// message arriving twice — and those two want opposite answers. That
/// judgement belongs where message identity exists, in `postio_app::reading`.
fn load_document(canvas: &Canvas<'_>, document: &str) {
    canvas.loads.set(canvas.loads.get() + 1);
    // The single choke point every render passes through, which is what makes
    // it the honest place to count from: a second path to the engine would
    // have to avoid this function to avoid the counter. `canvas.loads` beside
    // it is per-canvas and cannot be read from another crate's suite.
    postio_ui::reader::cost::note_render();
    canvas.document.replace(document.to_owned());
    let place = canvas.place;
    place.shown.replace(document.to_owned());
    let content = content_for(document, place);
    // A redraw of what is on screen keeps the reader's place; anything else
    // starts at the top.
    if place.keep.replace(false) {
        canvas.view.set_content(content);
    } else {
        canvas.view.set_content_from_top(content);
    }
    fetch_remote(canvas.view, place, document);
}

/// A conversation's plain-text alternative, which a render past its deadline
/// shows instead (spec 006 FR-023): each message under who sent it and when,
/// in the order the document draws them, with its own text part when it has
/// one.
fn thread_plain_text(messages: &[ThreadMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let text = message
                .body
                .text
                .as_deref()
                .filter(|_| !message.absent)
                .map(str::trim_end)
                .unwrap_or_default();
            format!("{} · {}\n{text}", message.sender, message.when)
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// What the view is handed for `document`: its parts, faces and whatever
/// remote images have arrived for it.
fn content_for(document: &str, place: &Place) -> crate::body_view::Content {
    let resources = resources_for(document, &*place.source);
    let remote = place.remote.borrow();
    for url in remote_image_urls(document) {
        if let Some(bytes) = remote.get(&url) {
            resources.insert_remote(&url, bytes.clone());
        }
    }
    crate::body_view::Content {
        document: if place.flow.get() {
            flow_document(document)
        } else {
            document.to_owned()
        },
        resources: std::sync::Arc::new(resources),
        plain_text: place.plain.borrow().clone(),
        over_cap: None,
    }
}

/// The rules a column that scrolls as one adds to every document
/// ([`Reader::flow_in`]): the ground is the column's own, so the body needs
/// no page of its own; and the body's frame goes, as does the padding
/// inside it, with the text at the reading size and the column's measure: the
/// column decides how wide a line runs (Focus's, T197), so the body's
/// lines and the cards above them share their edges. Mail that paints
/// its own page -- the sender's sheet, or a page background the sender set --
/// keeps the frame, hairline and quiet, because its edge is the only thing
/// telling it apart from the column.
///
/// The ground is `--flow-ground`, which the view defines at each render
/// from the column's own ground token (`BodyView::set_ground`): no colour is
/// written here, so the body and the dialog around it cannot disagree, in
/// light or dark (T203; it was two hex values, and dark's was off by a
/// shade of libadwaita's).
const FLOW_CSS: &str = "\nbody { background: var(--flow-ground); padding: 0; }\n";

/// What is added for correspondence, which has no page of its own.
///
/// The body fills its owner's column, which is the measure: Focus's message
/// dialog sizes the column to 32em of the 15px reading size, about 70
/// characters (T207), and centres it, so the body's lines share both edges
/// with the blocks above them. The rhythm inside the body is the handoff's
/// (T208): a 24px line, 12px between paragraphs -- a plain-text paragraph
/// break is a gap, not an empty line -- and list items 4px apart under a
/// 20px indent. The last block keeps no gap below it, so what follows the
/// body measures its own distance from the last line.
const FLOW_FLAT_CSS: &str = "body { font-size: 15px; line-height: 1.6; }\n\
    .postio-body { padding: 0; border: 0; border-radius: 0; min-height: 0; }\n\
    pre.postio-body-text, p { margin: 0 0 0.8em 0; }\n\
    ul, ol { margin: 0 0 0.8em 0; padding-left: 20px; }\n\
    li + li { margin-top: 4px; }\n\
    pre.postio-body-text:last-child, p:last-child { margin-bottom: 0; }\n";

/// The reader palette variables a flowing column supplies from its own
/// tokens, and the class of the probe each is read from: the owner's
/// stylesheet sets each probe's `color` to the token (Focus's `focus.css`).
/// A probe nobody styles reads as the toolkit's foreground, which is the
/// ink -- a safe answer for every one of them but the ground.
pub const FLOW_PALETTE: [(&str, &str); 6] = [
    ("--r-ink", "postio-flow-ink"),
    ("--r-ink-secondary", "postio-flow-ink-secondary"),
    ("--r-dim", "postio-flow-dim"),
    ("--r-accent", "postio-flow-accent"),
    ("--r-hairline", "postio-flow-hairline"),
    ("--r-hairline-strong", "postio-flow-hairline-strong"),
];

/// `document` as the column draws it.
fn flow_document(document: &str) -> String {
    let own_page = document.contains("class=\"postio-canvas\"")
        || document.contains(&format!(
            "class=\"{}\"",
            postio_ui::reader::document::SENDERS_SHEET_CLASS
        ))
        || document.contains(
            &postio_ui::reader::document::contain_body_treated("", Treatment::Paper)
                .replace("</div>", ""),
        );
    let mut css = String::from(FLOW_CSS);
    if !own_page {
        css.push_str(FLOW_FLAT_CSS);
    }
    document.replacen("</style>", &format!("{css}</style>"), 1)
}

/// Ask the owner's fetcher for the remote images `document` names that
/// have not been asked for yet, and redraw once when they arrive.
///
/// A document names a remote image only when its sender is allowed or the
/// user chose "Show once": the sanitizer strips every one otherwise
/// (spec 006 FR-025). And only for a message the user opened: the pane
/// follows the list's cursor, so a message counts as opened once it has
/// stayed on screen for the dwell that marks it read ([`DWELL_TO_READ`]) --
/// a cursor sweeping past it fetches nothing -- or at once when the load
/// is the user's own consent.
///
/// [`DWELL_TO_READ`]: postio_ui::dwell::DWELL_TO_READ
fn fetch_remote(view: &crate::body_view::BodyView, place: &Rc<Place>, document: &str) {
    if remote_image_urls(document).is_empty() {
        return;
    }
    if place.consented.replace(false) {
        ask_remote(view, place, document);
        return;
    }
    let view = view.downgrade();
    let weak = Rc::downgrade(place);
    let document = document.to_owned();
    glib::timeout_add_local_once(postio_ui::dwell::DWELL_TO_READ, move || {
        let (Some(view), Some(place)) = (view.upgrade(), weak.upgrade()) else {
            return;
        };
        if *place.shown.borrow() == document {
            ask_remote(&view, &place, &document);
        }
    });
}

/// The asking half of [`fetch_remote`], once the message counts as opened.
fn ask_remote(view: &crate::body_view::BodyView, place: &Rc<Place>, document: &str) {
    let Some(fetch) = place.fetch.borrow().clone() else {
        return;
    };
    let wanted: Vec<String> = {
        let mut asked = place.asked.borrow_mut();
        remote_image_urls(document)
            .into_iter()
            .filter(|url| asked.insert(url.clone()))
            .collect()
    };
    if wanted.is_empty() {
        return;
    }
    let view = view.downgrade();
    let weak = Rc::downgrade(place);
    let document = document.to_owned();
    fetch(
        wanted,
        Box::new(move |arrived| {
            let (Some(view), Some(place)) = (view.upgrade(), weak.upgrade()) else {
                return;
            };
            if arrived.is_empty() {
                return;
            }
            place.remote.borrow_mut().extend(arrived);
            // One redraw for the whole batch, and only of the document it
            // was asked for: a message the user has moved on from is not
            // drawn again behind their back.
            if *place.shown.borrow() == document {
                view.set_content(content_for(&document, &place));
            }
        }),
    );
}

/// What "View source" draws: each message's body as it was sent -- its
/// HTML, or its text -- escaped into a document of preformatted text, with
/// nothing in it that could load or run.
fn source_document(open: Option<&Open>, thread: &[ThreadMessage]) -> String {
    let bodies: Vec<&MessageBody> = match open {
        Some(open) => vec![&open.body],
        None => thread.iter().map(|message| &message.body).collect(),
    };
    let mut out = String::new();
    for body in bodies {
        let source = body.html.as_deref().or(body.text.as_deref()).unwrap_or("");
        out.push_str("<pre class=\"postio-source\" style=\"white-space: pre-wrap\">");
        for c in source.chars() {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '&' => out.push_str("&amp;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
        out.push_str("</pre>");
    }
    wrap_document(&out, RemoteImages::Blocked, Sheet::Theme)
}

/// The `http` and `https` URLs `document` names as images: an `<img>`'s
/// `src`, a `background=` attribute, and a background in a style. Never a
/// link's `href`, a cursor or anything else CSS can name: the contract
/// allows images and backgrounds and nothing more.
fn remote_image_urls(document: &str) -> Vec<String> {
    let decode = |url: &str| {
        url.replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    };
    let mut urls: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if (url.starts_with("http://") || url.starts_with("https://")) && !urls.contains(&url) {
            urls.push(url);
        }
    };
    for attribute in ["src=\"", "background=\""] {
        for piece in document.split(attribute).skip(1) {
            if let Some(end) = piece.find('"') {
                push(decode(&piece[..end]));
            }
        }
    }
    let mut pieces = document.split("url(");
    let mut before = pieces.next().unwrap_or("");
    for piece in pieces {
        // The declaration this `url(` is in: from the last boundary before
        // it. Only a background names an image.
        let declaration = before
            .rfind([';', '{', '"', '\''])
            .map_or(before, |at| &before[at + 1..]);
        let is_background = declaration.trim_start().starts_with("background");
        before = piece;
        if !is_background {
            continue;
        }
        let piece = decode(piece);
        let piece = piece.trim_start();
        let (quote, rest) = match piece.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &piece[1..]),
            _ => (None, piece),
        };
        let end = match quote {
            Some(q) => rest.find(q),
            None => rest.find(')'),
        };
        if let Some(end) = end {
            push(rest[..end].trim().to_owned());
        }
    }
    urls
}

/// Every part and face `document` may load, resolved now, on this thread:
/// the render thread cannot call `source`, and asks for nothing it was not
/// given (spec 006 FR-003).
fn resources_for(document: &str, source: &dyn BlobSource) -> postio_render::Resources {
    let resources = postio_render::Resources::new();
    for face in postio_ui::reader::document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    let prefix = format!("{}:", postio_body::CID_SCHEME);
    let mut rest = document;
    while let Some(at) = rest.find(&prefix) {
        rest = &rest[at + prefix.len()..];
        let end = rest
            .find(['"', '\'', ')', ' ', '>', '<'])
            .unwrap_or(rest.len());
        let reference = &rest[..end];
        let (scope, id) = match reference.split_once('/') {
            Some((scope, id)) => (Some(scope), id),
            None => (None, reference),
        };
        let id = postio_body::sanitize::percent_decode(id);
        if let Some((bytes, _)) = source.resolve_in(scope, &id) {
            resources.insert_part(scope, &id, bytes);
        }
        rest = &rest[end..];
    }
    resources
}

/// The whole thread as one document.
///
/// A free function over the allow list rather than a method, for the reason
/// `render_open` is one: the `Show` verb inside the document has to be able
/// to re-render after granting consent, and a closure that held the whole
/// `Reader` to do it would hold the widget that owns the closure.
fn compose_thread_document(
    messages: &[ThreadMessage],
    allowlist: &RefCell<RemoteImageAllowList>,
    originals: &std::collections::HashMap<String, Rendering>,
    renders: &RefCell<postio_ui::reader::document::RenderCache>,
) -> String {
    // Rendered first, and held, because `Entry` borrows the markup.
    // Every message opens as its sender built it (spec 006 FR-031), exactly
    // as `render` decides it for one.
    let rendered: Vec<postio_ui::reader::document::Rendered> = messages
        .iter()
        .map(|message| {
            // The reader's own choice first -- `⌃O` or reader view on one
            // message, for that message and no other (#1398) -- and how every
            // message opens otherwise.
            let rendering = originals
                .get(&message.scope)
                .copied()
                .unwrap_or_else(postio_ui::reader::document::opening_rendering);
            // Per **message**, from its own sender. A conversation holds
            // several and the decision is per sender (`PRODUCT.md` §21),
            // so one allowed correspondent must not carry the rest of the
            // thread with them. This asked for `Blocked` unconditionally,
            // which threw away a decision the user had already made the
            // moment the message appeared in a conversation (#1353).
            let remote = if allowlist.borrow().is_allowed(&message.address) {
                RemoteImages::Allowed
            } else {
                RemoteImages::Blocked
            };
            if message.absent && message.expanded {
                // The same words the single-message pane has always used, not
                // a second way of saying it -- `absent_html` carries the
                // `role="status"` live region with it, so a screen reader is
                // told when the body is still coming and told it once.
                //
                // `Partial` rather than a state read per message: the thread
                // knows only that no body is here yet, which is what `Partial`
                // means. Offline is said by the connection banner, which is
                // about the account and not about one message.
                //
                // Only when it is open. A collapsed message contributes its
                // section to the document either way, so emitting the plate
                // for all of them puts thirty copies of one sentence into a
                // thirty-message thread -- invisible, but each carrying an
                // `aria-live` region, which is not invisible to a screen
                // reader. A collapsed message is its one preview line, and
                // that line is built from headers, which are here.
                return postio_ui::reader::document::Rendered {
                    html: postio_ui::reader::document::absent_html(
                        postio_ui::reader::document::Absent::Partial,
                    ),
                    ..postio_ui::reader::document::Rendered::default()
                };
            }
            renders
                .borrow_mut()
                .render(&message.scope, &message.body, remote, rendering)
        })
        .collect();
    renders
        .borrow_mut()
        .keep_only(messages.iter().map(|message| message.scope.as_str()));
    let entries: Vec<postio_ui::reader::thread::Entry<'_>> = messages
        .iter()
        .zip(&rendered)
        .map(|(message, rendered)| postio_ui::reader::thread::Entry {
            scope: &message.scope,
            sender: &message.sender,
            address: &message.address,
            when: &message.when,
            preview: &message.preview,
            expanded: message.expanded,
            latest: message.latest,
            draft: message.draft,
            mine: message.mine,
            blocked: rendered.held_back.remote_images,
            body: &rendered.html,
            styles: &rendered.styles,
            recipients: &message.recipients,
            cc: &message.cc,
        })
        .collect();

    // The document's `Content-Security-Policy` is one policy for the whole
    // page, and there is no per-message form of it -- which is exactly the
    // limitation ADR 0032 names: "a document-level network policy cannot
    // express [per-sender], so the distinction has to move into how each
    // message's images are addressed".
    //
    // So it opens only when some message in the thread is from a sender
    // the user allowed, and the *sanitizer* is what keeps the others out:
    // a blocked sender's `src` is dropped before the markup is composed,
    // and the assertion in `gtk_reader` that a stranger's image is absent
    // is what holds that line.
    //
    // Worth saying plainly rather than leaving implied: for such a
    // document the CSP is no longer a second, independent refusal. It is
    // still the only refusal for every thread where nobody is allowed,
    // which is the ordinary case.
    let anyone_allowed = messages
        .iter()
        .any(|message| allowlist.borrow().is_allowed(&message.address));
    postio_ui::reader::thread::conversation_document(
        &entries,
        if anyone_allowed {
            RemoteImages::Allowed
        } else {
            RemoteImages::Blocked
        },
        postio_ui::reader::document::Sheet::Theme,
    )
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
    let (drawn, sender, bulk) = {
        let guard = open.borrow();
        let Some(current) = guard.as_ref() else {
            return;
        };
        // What was sanitised off the main thread, when it was for exactly
        // this; the sanitiser here otherwise, as it always was.
        let drawn = if canvas.place.treatments.get() {
            postio_ui::reader::document::body_html_treated(
                &current.body,
                remote,
                current.chosen,
                None,
            )
        } else {
            match current.prepared.as_ref() {
                Some(prepared) if prepared.serves(&current.body, remote, current.rendering) => {
                    prepared.rendered().clone()
                }
                _ => body_html(&current.body, remote, current.rendering),
            }
        };
        (drawn, current.sender.clone(), current.bulk)
    };
    let remembered = open
        .borrow()
        .as_ref()
        .and_then(|current| current.remembered);
    let held_back = drawn.held_back;
    let content = drawn.html.clone();
    // After sanitizing and quote-folding, never before: ammonia would strip
    // the `<mark>` as an unknown tag, and there is no point running a matcher
    // over markup that has not been cleaned yet.
    let content = postio_ui::search::mark_html(&content, &highlight.borrow());

    banner.set_sender(sender.as_deref());
    // The count, before the visibility: a notice that appeared and then
    // changed what it said would flicker a number at the reader (#1008).
    banner.set_held_back(held_back);
    canvas.notices.want(
        Notice::RemoteImages,
        remote == RemoteImages::Blocked && held_back.total() > 0,
    );

    // Only while a message is actually drawn reduced. A notice offering to
    // show an original that is already on screen is a control that does
    // nothing, which is worse than no control.
    canvas
        .notices
        .want(Notice::ReaderView, drawn.rendering == Rendering::Reader);
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
    let document = match drawn.treated {
        Some(treated) => postio_ui::reader::document::document_for_treated(
            &content,
            &drawn.styles,
            remote,
            treated.shown,
        ),
        None => document_for(&content, &drawn.styles, remote, sheet),
    };
    canvas.place.treated.set(drawn.treated);
    show_treatment(canvas.place, remembered);
    load_document(canvas, &document);

    for handler in rendered.borrow().iter() {
        handler(held_back);
    }
    if let Some(treated) = drawn.treated {
        let handlers = canvas.place.on_treatment.borrow().clone();
        for handler in handlers {
            handler(treated.shown);
        }
    }
}

/// Put the render-mode line in step with what is drawn: the words for
/// `place`'s treatment, or no line.
fn show_treatment(place: &Place, remembered: Option<Treatment>) {
    if let Some(line) = place.line.borrow().as_ref() {
        line.show(place.treated.get().and_then(|treated| {
            postio_ui::reader::document::render_mode_words(treated, remembered)
        }));
    }
}

fn collect_labels(widget: &gtk::Widget, found: &mut Vec<String>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && !label.has_css_class("postio-keyhint")
    {
        let text = label.text().to_string();
        if !text.is_empty() {
            found.push(text);
        }
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        child = node.next_sibling();
        collect_labels(&node, found);
    }
}

#[cfg(test)]
mod tests {
    /// A body in app colours is flat in the column; one on paper keeps its
    /// sheet. The stylesheet names both treatments, so the test is the
    /// container's own stamp, not the attribute anywhere in the document.
    #[test]
    fn the_column_flattens_app_colours_and_keeps_papers_sheet() {
        use postio_ui::reader::document::document_for_treated;
        let flat = |treatment| {
            super::flow_document(&document_for_treated(
                "<p>x</p>",
                "",
                super::RemoteImages::Blocked,
                treatment,
            ))
            .contains(super::FLOW_FLAT_CSS)
        };
        assert!(
            flat(super::Treatment::AppColours),
            "app colours kept a frame"
        );
        assert!(!flat(super::Treatment::Paper), "paper lost its sheet");
    }

    /// The column's ground is a token resolved at render time, never a
    /// colour written into the sheet the column adds (T203).
    #[test]
    fn the_flow_sheet_names_its_ground_and_writes_no_colour() {
        let sheet = format!("{}{}", super::FLOW_CSS, super::FLOW_FLAT_CSS);
        assert!(sheet.contains("var(--flow-ground)"), "{sheet}");
        assert!(
            !sheet.contains('#') && !sheet.contains("rgb") && !sheet.contains("--flow-ground:"),
            "the flow sheet writes a colour of its own: {sheet}"
        );
    }

    /// Only what a document names as an image is fetched: a link's
    /// target is the user's to follow (spec 006 FR-025).
    #[test]
    fn remote_image_urls_are_images_and_styles_never_links() {
        let document = r#"<a href="https://example.net/page">x</a>
            <img src="https://images.example.net/a.gif?x=1&amp;y=2">
            <img src="postio-cid:part@example.com">
            <div style="background:url(&quot;http://images.example.net/b.png&quot;)"></div>
            <style>.hero { background: url(https://images.example.net/c.png) }
            .pointer { cursor: url(https://images.example.net/cursor.png), auto }
            li { list-style-image: url(https://images.example.net/bullet.png) }</style>
            <table background="https://images.example.net/d.png"></table>
            <img src="https://images.example.net/a.gif?x=1&amp;y=2">"#;
        assert_eq!(
            super::remote_image_urls(document),
            [
                "https://images.example.net/a.gif?x=1&y=2",
                "https://images.example.net/d.png",
                "http://images.example.net/b.png",
                "https://images.example.net/c.png",
            ]
        );
    }

    #[test]
    fn the_two_schemes_do_not_share_a_ground() {
        // A palette that collapsed to one value would paint a white flash
        // into dark mode and pass a test that only checked parseability.
        assert_ne!(
            postio_ui::reader::document::reader_ground(false),
            postio_ui::reader::document::reader_ground(true),
            "both schemes report the same ground, so one of them is painting \
             the wrong colour between messages"
        );
    }

    use super::*;

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
