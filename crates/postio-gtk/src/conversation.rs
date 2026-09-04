//! The conversation pane: every message of a thread, stacked (ADR 0015 Q4).
//!
//! The reading pane used to show one message. Opening a conversation now
//! fills it with all of them, oldest first, read ones collapsed to a one-line
//! header and the rest expanded onto the reader Postio already has.
//!
//! # There is one surface, and this is it
//!
//! A drill-in column used to list the same messages in the pane the message
//! list occupies, cast as a table of contents into this one (#1003). It is
//! gone: the list is only ever the list, and a conversation lives here and
//! nowhere else. Nothing has to be kept in step with anything.
//!
//! # Why the policy is pure and lives at the top of this file
//!
//! Where focus opens and how much expands are the two decisions with real
//! consequences — one for whether the pane lands where you stopped reading,
//! the other for whether a thirty-message conversation instantiates thirty
//! `WebKitWebView`s. Both are worth testing without a display, so both are
//! functions over rows rather than behaviour buried in a widget.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::ids::MessageId;

use crate::list::Row;

/// How many messages open expanded at most.
///
/// Every expanded message is a `WebKitWebView`, and "expand everything
/// unread" over a conversation nobody has read is one per message — which
/// holds neither the interaction budget nor the memory. Three is what a
/// person reads before they scroll, and scrolling expands more.
pub const EAGER_EXPANSION_CAP: usize = 3;

/// How a conversation orders its messages.
///
/// Was `crate::thread::Order`, when the drill-in column offered `o` to
/// reverse it (#1003). The column is gone and so is the key: a conversation
/// stacks oldest first, the way it was had. The type stays because the
/// ordering itself is still a decision, and one worth being able to state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Oldest first — how a conversation was actually had, and how the pane
    /// stacks it.
    #[default]
    Oldest,
    /// Newest first, matching the message list.
    Newest,
}

/// The rows a conversation shows, given what is in it and how it is ordered.
///
/// Pure, and tested without a display: the ordering is the part worth being
/// sure about, and it has nothing to do with GTK.
pub fn arrange(rows: &[Row], order: Order, unread_only: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = rows
        .iter()
        .filter(|row| !unread_only || !row.seen)
        .cloned()
        .collect();
    // By id after the timestamp, so two messages that claim the same second —
    // a sender and their own auto-reply, commonly — do not swap places
    // between one redraw and the next.
    rows.sort_by_key(|row| (row.received_at, row.id));
    if order == Order::Newest {
        rows.reverse();
    }
    rows
}

/// How many distinct people are in a conversation.
///
/// By address, folded: one correspondent who has changed their display name
/// mid-thread is still one person, and the header's count is a count of
/// correspondents rather than of `From` headers.
pub fn people(rows: &[Row]) -> usize {
    let mut seen: Vec<String> = rows
        .iter()
        .filter_map(|row| row.from.as_ref())
        .map(|from| from.address.to_lowercase())
        .collect();
    seen.sort();
    seen.dedup();
    seen.len()
}

/// Which message the pane opens on.
///
/// **The first unread**, not the newest. A conversation you open is one you
/// are part way through, and landing at the end means scrolling back past
/// everything you have already read. When every message has been read there
/// is no first unread and the newest is what you came back for.
///
/// `None` only for an empty conversation, which the pane does not draw.
///
/// `messages` is oldest first, which is the order the pane stacks them in.
pub fn opening_focus(messages: &[Row]) -> Option<usize> {
    if messages.is_empty() {
        return None;
    }
    messages
        .iter()
        .position(|message| !message.seen)
        .or(Some(messages.len() - 1))
}

/// Which messages are expanded when the conversation opens.
///
/// Read messages are collapsed: they are one line, and collapsing them is
/// what makes a long conversation readable at all. From the focused message
/// onwards the unread ones expand, because that is the part being read — up
/// to `cap`, after which the rest stay one keystroke away rather than costing
/// a web view each.
///
/// The focused message always expands, even when it has been read: focus
/// means "this is the one you are looking at", and looking at a one-line
/// header is not reading.
pub fn expanded_on_open(messages: &[Row], focus: usize, cap: usize) -> Vec<bool> {
    let mut expanded = vec![false; messages.len()];
    let mut spent = 0;
    for (index, message) in messages.iter().enumerate().skip(focus) {
        if spent >= cap {
            break;
        }
        if index == focus || !message.seen {
            expanded[index] = true;
            spent += 1;
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use postio_model::ids::MessageId;

    /// A message in the conversation, read or not.
    fn message(id: i64, seen: bool) -> Row {
        Row {
            id: MessageId::new(id),
            thread: None,
            from: None,
            subject: None,
            preview: None,
            received_at: Utc.timestamp_opt(1_770_000_000 + id, 0).single().unwrap(),
            seen,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
            participants: Vec::new(),
        }
    }

    // -- where the pane opens ---------------------------------------------

    #[test]
    fn a_conversation_opens_on_its_first_unread_message() {
        // The whole point of the rule: two read, then the one you stopped at.
        let messages = [
            message(1, true),
            message(2, true),
            message(3, false),
            message(4, false),
        ];
        assert_eq!(opening_focus(&messages), Some(2));
    }

    #[test]
    fn a_conversation_read_all_the_way_through_opens_on_its_newest() {
        // There is no first unread, and the end is what you came back for.
        let messages = [message(1, true), message(2, true), message(3, true)];
        assert_eq!(opening_focus(&messages), Some(2));
    }

    #[test]
    fn a_wholly_unread_conversation_opens_at_the_beginning() {
        // Not at the newest: this is a conversation you have never read, and
        // reading it from the end backwards is not how anyone reads.
        let messages = [message(1, false), message(2, false), message(3, false)];
        assert_eq!(opening_focus(&messages), Some(0));
    }

    #[test]
    fn an_unread_message_older_than_a_read_one_still_wins() {
        // Read state is not monotonic: someone can mark a later message
        // unread, or read out of order. "First unread" means first, not
        // "first after the last read one".
        let messages = [message(1, true), message(2, false), message(3, true)];
        assert_eq!(opening_focus(&messages), Some(1));
    }

    #[test]
    fn an_empty_conversation_has_nowhere_to_focus() {
        assert_eq!(opening_focus(&[]), None);
    }

    // -- what opens expanded ----------------------------------------------

    #[test]
    fn everything_before_the_focus_stays_collapsed() {
        // Read messages are one line. That is what makes a long conversation
        // readable rather than a wall.
        let messages = [
            message(1, true),
            message(2, true),
            message(3, false),
            message(4, false),
        ];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![false, false, true, true]);
    }

    #[test]
    fn a_long_unread_conversation_does_not_expand_all_of_it() {
        // The cost question. Thirty unread messages is thirty web views, and
        // the cap is what stops the pane from opening one per message.
        let messages: Vec<Row> = (0..30).map(|id| message(id, false)).collect();
        let expanded = expanded_on_open(&messages, 0, EAGER_EXPANSION_CAP);

        assert_eq!(
            expanded.iter().filter(|open| **open).count(),
            EAGER_EXPANSION_CAP,
            "opening a conversation must not cost a web view per message"
        );
        assert!(
            expanded[..EAGER_EXPANSION_CAP].iter().all(|open| *open),
            "the ones that do expand are the ones being read, from the focus \
             forward"
        );
    }

    #[test]
    fn the_focused_message_expands_even_when_it_has_been_read() {
        // Focus means "this is the one you are looking at", and looking at a
        // one-line header is not reading. This is the fully-read case: focus
        // lands on the newest and it has to open.
        let messages = [message(1, true), message(2, true), message(3, true)];
        let expanded = expanded_on_open(&messages, 2, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![false, false, true]);
    }

    #[test]
    fn a_read_message_after_the_focus_stays_collapsed() {
        // Only the focus is expanded unconditionally; past it, unread is what
        // earns a web view.
        let messages = [message(1, false), message(2, true), message(3, false)];
        let expanded = expanded_on_open(&messages, 0, EAGER_EXPANSION_CAP);
        assert_eq!(expanded, vec![true, false, true]);
    }

    #[test]
    fn a_cap_of_one_opens_only_what_is_focused() {
        // The fallback shape ADR 0015 names if the stack proves too
        // expensive: one reader, the rest collapsed.
        let messages: Vec<Row> = (0..5).map(|id| message(id, false)).collect();
        let expanded = expanded_on_open(&messages, 1, 1);
        assert_eq!(expanded, vec![false, true, false, false, false]);
    }

    #[test]
    fn an_empty_conversation_expands_nothing() {
        assert!(expanded_on_open(&[], 0, EAGER_EXPANSION_CAP).is_empty());
    }
}

/// What the pane asks for when a message needs a body: the reader for that
/// message.
///
/// A callback rather than a constructor argument, because building a
/// [`crate::reader::Reader`] needs a blob source and an allow-list path that
/// only the window has — and because it is what makes the cost testable. A
/// test hands back a bare reader and counts the calls; the application hands
/// back the hardened one.
///
/// Hands back the [`Reader`](crate::reader::Reader) itself, not just its
/// widget: the pane keeps it, so an arrival for an already-expanded entry
/// can be re-drawn into the reader already on screen ([`reader_for`],
/// #739) instead of tearing the whole entry down to rebuild one.
///
/// [`reader_for`]: ConversationView::reader_for
pub type ReaderFactory = Box<dyn Fn(MessageId) -> crate::reader::Reader>;

/// The three verbs a single message in a stack offers.
///
/// Reply is the primary: it is what the pane is for. Archive and delete are
/// deliberately absent — every verb but these three is the conversation's
/// (ADR 0015 Q4), and a delete button on every message in a stack is how
/// people delete the wrong one.
pub const MESSAGE_ACTIONS: [crate::widgets::Action; 3] = [
    crate::widgets::Action::new(
        postio_core::CommandId::Reply,
        "Reply",
        "conversation-action-reply",
    )
    .primary(),
    crate::widgets::Action::new(
        postio_core::CommandId::ReplyAll,
        "Reply all",
        "conversation-action-reply-all",
    ),
    crate::widgets::Action::new(
        postio_core::CommandId::Forward,
        "Forward",
        "conversation-action-forward",
    ),
];

/// The conversation's own verbs, drawn in the footer.
///
/// Reply, reply-all and forward are per *message* and live inside each entry
/// ([`MESSAGE_ACTIONS`]); everything else is the conversation's (ADR 0015
/// Q4). `Reply to conversation` is the same command `e` runs and aims at the
/// same message — the focused one — so the button and the key cannot
/// diverge.
pub const CONVERSATION_ACTIONS: [crate::widgets::Action; 2] = [
    crate::widgets::Action::new(
        postio_core::CommandId::Reply,
        "Reply to conversation",
        "conversation-footer-reply",
    )
    .primary(),
    crate::widgets::Action::new(
        postio_core::CommandId::ArchiveThread,
        "Archive thread",
        "conversation-footer-archive",
    ),
];

/// The pane's own header: what conversation this is, and how much of it.
///
/// Subject at the largest size in the pane — this is the one place the
/// conversation is named, and before the drill-in column went there were two
/// places and they could disagree. Under it one metadata line, ellipsised
/// rather than wrapped, and the way to open everything at once.
pub struct Header {
    root: gtk::Box,
    subject: gtk::Label,
    meta: gtk::Label,
    expand_all: std::rc::Rc<crate::widgets::KeycapButton>,
}

impl Header {
    /// Build the header, empty.
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("conversation-header");
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);

        let subject = gtk::Label::new(None);
        subject.set_xalign(0.0);
        subject.set_wrap(false);
        subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
        subject.add_css_class("conversation-subject");
        titles.append(&subject);

        let meta = gtk::Label::new(None);
        meta.set_xalign(0.0);
        // One line, ellipsised. The participants are the unbounded part —
        // a twelve-person thread must not grow the header.
        meta.set_wrap(false);
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        meta.add_css_class("conversation-meta");
        titles.append(&meta);
        root.append(&titles);

        let expand_all = std::rc::Rc::new(crate::widgets::KeycapButton::new(
            Some(postio_core::CommandId::ExpandAll),
            "Expand all",
            "conversation-expand-all",
            false,
        ));
        crate::widgets::KeycapButton::arm(&expand_all);
        root.append(&expand_all.widget());

        Header {
            root,
            subject,
            meta,
            expand_all,
        }
    }

    /// The widget to pin above the stack.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Name the conversation on screen.
    ///
    /// `rows` is the whole conversation, oldest first — the header counts and
    /// spans it rather than being told, so it cannot disagree with the stack
    /// below it about how many messages there are.
    pub fn set_conversation(&self, rows: &[Row], now: chrono::DateTime<chrono::Local>) {
        if rows.is_empty() {
            self.root.set_visible(false);
            return;
        }
        self.root.set_visible(true);
        self.subject.set_label(
            rows.iter()
                .find_map(|row| row.subject.as_deref())
                .filter(|subject| !subject.trim().is_empty())
                .unwrap_or("(no subject)"),
        );

        let senders: Vec<postio_model::address::EmailAddress> =
            rows.iter().filter_map(|row| row.from.clone()).collect();
        let first = rows
            .iter()
            .map(|row| row.received_at)
            .min()
            .unwrap_or_else(chrono::Utc::now);
        let last = rows
            .iter()
            .map(|row| row.received_at)
            .max()
            .unwrap_or_else(chrono::Utc::now);

        let meta = [
            postio_ui::conversation::message_count(rows.len()),
            postio_ui::conversation::participants(&senders),
            postio_ui::conversation::date_span(
                first.with_timezone(&chrono::Local),
                last.with_timezone(&chrono::Local),
                now,
            ),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        self.meta.set_label(&meta);
        // The line ellipsises, so the whole of it has to reach a screen
        // reader some other way.
        self.root
            .update_property(&[gtk::accessible::Property::Description(&meta)]);
    }

    /// What the metadata line currently says. Test-facing.
    pub fn meta(&self) -> String {
        self.meta.label().to_string()
    }

    /// What the subject line currently says. Test-facing.
    pub fn subject(&self) -> String {
        self.subject.label().to_string()
    }

    /// Re-cap `Expand all` from the live keymap.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        self.expand_all
            .set_key(keymap.binding(postio_core::CommandId::ExpandAll));
    }

    /// Called when `Expand all` is pressed.
    pub fn connect_expand_all(&self, handler: impl Fn() + 'static) {
        self.expand_all.connect_clicked(handler);
    }

    /// Press `Expand all` without a pointer, for a test.
    pub fn press_expand_all(&self) {
        self.expand_all.press();
    }
}

impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}

type MessageHandler = Box<dyn Fn(MessageId)>;
type ReplyHandler = Box<dyn Fn(MessageId, bool)>;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct ConversationView {
        /// The whole pane: header, scroller, footer, stacked.
        pub(super) root: gtk::Box,
        /// The conversation's own name and shape, pinned above the stack so
        /// it survives scrolling. Nothing else on screen says what you are
        /// reading once the drill-in column's header went (#1004).
        pub(super) header: super::Header,
        /// `Reply to conversation` and `Archive thread`, pinned below —
        /// where the conversation's own verbs live, as against the
        /// per-message ones inside each entry (#1006).
        pub(super) footer: std::rc::Rc<crate::widgets::ActionBar>,
        /// The scroller the stack lives in. A conversation is longer than the
        /// pane, and jumping to a message means scrolling this.
        pub(super) scroller: gtk::ScrolledWindow,
        /// The stack itself, one [`Entry`] per message, oldest first.
        pub(super) stack: gtk::Box,
        pub(super) entries: RefCell<Vec<Entry>>,
        /// The current message — the one both this pane and the drill-in
        /// column are showing, and the one a per-message verb aims at.
        pub(super) focused: Cell<Option<MessageId>>,
        pub(super) factory: RefCell<Option<ReaderFactory>>,
        pub(super) on_reply: RefCell<Vec<ReplyHandler>>,
        pub(super) on_forward: RefCell<Vec<MessageHandler>>,
        pub(super) on_focus: RefCell<Vec<MessageHandler>>,
        pub(super) on_dwell: RefCell<Vec<MessageHandler>>,
        /// The dwell timer in flight. Cancelled rather than replaced when
        /// focus moves: a `glib` timeout that merely loses its handle still
        /// fires, and one that fires late marks a message read that was
        /// passed over rather than looked at.
        pub(super) dwell: RefCell<Option<glib::SourceId>>,
        pub(super) dwell_delay: Cell<std::time::Duration>,
        /// The dividers currently standing in for folded runs, in stack
        /// order. Rebuilt whenever anything folds or unfolds, because
        /// collapsing a message between two runs joins them (#1005).
        /// Which entries a divider stands in for is not kept: `refold`
        /// recomputes every run from the collapsed flags each time, so a
        /// stored range would be a second source of truth that could only
        /// ever disagree with the first.
        pub(super) dividers: RefCell<Vec<gtk::Box>>,
    }

    /// One message in the stack: its header, and the body when it has one.
    pub struct Entry {
        pub message: MessageId,
        pub row: Row,
        /// The collapsed line, which is always drawn. Expanding adds a body
        /// beneath it rather than replacing it, so the header stays as the
        /// thing you click and the thing focus is drawn on.
        pub header: crate::thread_row::ThreadRowView,
        /// Where the reader goes. Empty until this message is expanded, which
        /// is what keeps a thirty-message conversation from costing thirty
        /// web views.
        pub body: gtk::Box,
        /// The reader built for this entry, once it is expanded — kept
        /// alongside `body` so an arrival for this message can be re-drawn
        /// into the same `WebView` rather than rebuilding it (`reader_for`,
        /// #739). `None` until `expand` fills it, and never cleared by
        /// `collapse`: the widget itself stays parked in `body`, hidden, for
        /// the same reason.
        pub reader: RefCell<Option<crate::reader::Reader>>,
        pub actions: std::rc::Rc<crate::widgets::ActionBar>,
        pub expanded: Cell<bool>,
        /// Whether a folded run this message was in has been shown.
        ///
        /// Distinct from `expanded`: `Show` on a divider puts the individual
        /// *collapsed* rows back, it does not open them (#1005). Without this
        /// the next `refold` would immediately fold the run again, because
        /// the messages are still collapsed and still consecutive.
        pub shown: Cell<bool>,
        /// The box holding header, actions and body — what the stack owns.
        pub container: gtk::Box,
    }

    impl Entry {
        /// The widget the stack holds for this message.
        pub fn container(&self) -> gtk::Box {
            self.container.clone()
        }
    }

    impl Default for ConversationView {
        fn default() -> Self {
            ConversationView {
                root: gtk::Box::new(gtk::Orientation::Vertical, 0),
                header: super::Header::new(),
                footer: crate::widgets::ActionBar::new(
                    &super::CONVERSATION_ACTIONS,
                    "conversation-footer",
                ),
                scroller: gtk::ScrolledWindow::default(),
                stack: gtk::Box::new(gtk::Orientation::Vertical, 0),
                entries: RefCell::new(Vec::new()),
                focused: Cell::new(None),
                factory: RefCell::new(None),
                on_reply: RefCell::new(Vec::new()),
                on_forward: RefCell::new(Vec::new()),
                on_focus: RefCell::new(Vec::new()),
                on_dwell: RefCell::new(Vec::new()),
                dwell: RefCell::new(None),
                dwell_delay: Cell::new(crate::list_view::DWELL_TO_READ),
                dividers: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ConversationView {
        const NAME: &'static str = "PostioConversationView";
        type Type = super::ConversationView;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_layout_manager_type::<gtk::BinLayout>();
            class.set_css_name("conversation");
        }
    }

    impl ObjectImpl for ConversationView {
        fn constructed(&self) {
            self.parent_constructed();
            let view = self.obj();
            self.scroller.set_child(Some(&self.stack));
            self.scroller.set_hexpand(true);
            self.scroller.set_vexpand(true);
            // A scroll area is a tab stop, and an unnamed one announces
            // nothing — see docs/engineering-notes.md.
            self.scroller
                .update_property(&[gtk::accessible::Property::Label("Conversation")]);

            // Header and footer are *outside* the scroller, deliberately: a
            // header that scrolled away would leave a long conversation with
            // nothing on screen saying what it is, and a footer that scrolled
            // away would put the conversation's own verbs somewhere you have
            // to go looking for.
            self.root.append(&self.header.widget());
            self.root.append(&self.scroller);
            self.root.append(&self.footer.widget());
            self.footer.set_visible(false);
            self.root.set_parent(&*view);
        }

        fn dispose(&self) {
            self.root.unparent();
        }
    }

    impl WidgetImpl for ConversationView {}
}

glib::wrapper! {
    /// Every message of one conversation, stacked.
    pub struct ConversationView(ObjectSubclass<imp::ConversationView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ConversationView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ConversationView {
    /// An empty pane.
    pub fn new() -> Self {
        let pane: Self = Self::default();
        // The header's button and `O` are the same verb; wiring the button to
        // the pane rather than emitting a command keeps them one path.
        pane.imp().header.connect_expand_all({
            let pane = pane.clone();
            move || pane.expand_all()
        });
        pane
    }

    /// This pane as a widget, for mounting into the reading pane.
    pub fn widget(&self) -> gtk::Widget {
        self.clone().upcast()
    }

    /// How the pane builds the body of an expanded message.
    ///
    /// Set once, by whoever owns the reader's blob source. Until it is set the
    /// pane draws headers and no bodies, which is what a pane with nothing
    /// wired to it should look like rather than a crash.
    pub fn set_reader_factory(
        &self,
        factory: impl Fn(MessageId) -> crate::reader::Reader + 'static,
    ) {
        *self.imp().factory.borrow_mut() = Some(Box::new(factory));
    }

    /// Put a conversation in the pane, oldest first.
    ///
    /// Focus lands on the first unread — see [`opening_focus`] — and
    /// [`expanded_on_open`] decides how much opens with it.
    pub fn open(&self, messages: Vec<Row>) {
        let imp = self.imp();
        for entry in imp.entries.borrow().iter() {
            imp.stack.remove(&entry.container());
        }
        imp.entries.borrow_mut().clear();
        for divider in imp.dividers.borrow_mut().drain(..) {
            imp.stack.remove(&divider);
        }
        imp.focused.set(None);
        self.cancel_dwell();

        imp.header.set_conversation(&messages, chrono::Local::now());
        imp.footer.set_visible(!messages.is_empty());

        let focus = opening_focus(&messages);
        let expanded = match focus {
            Some(focus) => expanded_on_open(&messages, focus, EAGER_EXPANSION_CAP),
            None => Vec::new(),
        };

        for (index, row) in messages.iter().enumerate() {
            // Numbered from one. Nothing draws the number since #1003 took
            // the column away, but a screen reader still says "3 of 8", which
            // is the position a sighted reader gets from the stack itself.
            let entry = self.build_entry(row, index as u32 + 1);
            imp.stack.append(&entry.container());
            imp.entries.borrow_mut().push(entry);
            if expanded.get(index).copied().unwrap_or(false) {
                self.expand(row.id);
            }
        }
        if let Some(focus) = focus.and_then(|index| messages.get(index)) {
            self.focus_message(focus.id);
        }
        self.refold();
    }

    /// Fold every run of three-or-more collapsed messages into one divider,
    /// and unfold any that no longer qualifies.
    ///
    /// Recomputed from scratch rather than patched, because the runs are not
    /// independent: collapsing the message between two runs joins them into
    /// one, and expanding inside a run splits it in two. Patching that
    /// correctly is harder than recounting, and the count is over a slice of
    /// bools — [`postio_ui::conversation::collapsed_runs`] — which is cheap
    /// and provable without a display.
    fn refold(&self) {
        let imp = self.imp();
        for divider in imp.dividers.borrow_mut().drain(..) {
            imp.stack.remove(&divider);
        }

        let collapsed: Vec<bool> = imp
            .entries
            .borrow()
            .iter()
            .map(|entry| !entry.expanded.get() && !entry.shown.get())
            .collect();
        let runs = postio_ui::conversation::collapsed_runs(
            &collapsed,
            postio_ui::conversation::RUN_MINIMUM,
        );

        for range in runs {
            let (senders, first) = {
                let entries = imp.entries.borrow();
                let senders: Vec<postio_model::address::EmailAddress> = entries[range.clone()]
                    .iter()
                    .filter_map(|entry| entry.row.from.clone())
                    .collect();
                (senders, entries[range.start].container())
            };
            let divider = self.build_divider(range.clone(), &senders);
            imp.stack
                .insert_child_after(&divider, first.prev_sibling().as_ref());
            for entry in imp.entries.borrow()[range.clone()].iter() {
                entry.container().set_visible(false);
            }
            imp.dividers.borrow_mut().push(divider);
        }
    }

    /// The hairline row standing in for one folded run.
    fn build_divider(
        &self,
        range: std::ops::Range<usize>,
        senders: &[postio_model::address::EmailAddress],
    ) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("conversation-divider");

        let rule = || {
            let line = gtk::Separator::new(gtk::Orientation::Horizontal);
            line.set_hexpand(true);
            line.set_valign(gtk::Align::Center);
            line
        };
        row.append(&rule());

        let label = gtk::Label::new(Some(&postio_ui::conversation::run_summary(
            range.len(),
            senders,
        )));
        label.add_css_class("conversation-divider-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&label);

        // `Show`, not `Expand`: it puts the individual collapsed rows back,
        // it does not open them. One gesture, one step -- expanding five
        // messages because you wanted to see who they were from is not what
        // anybody asked for.
        let show = gtk::Button::with_label("Show");
        show.add_css_class("flat");
        show.add_css_class("postio-ghost");
        show.add_css_class("conversation-divider-show");
        let view = self.clone();
        let range_for_show = range.clone();
        show.connect_clicked(move |_| view.show_run(range_for_show.clone()));
        row.append(&show);
        row.append(&rule());

        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{}, collapsed",
            postio_ui::conversation::run_summary(range.len(), senders)
        ))]);

        row
    }

    /// Put a folded run's individual rows back, still collapsed.
    pub fn show_run(&self, range: std::ops::Range<usize>) {
        {
            let imp = self.imp();
            let entries = imp.entries.borrow();
            for entry in entries.get(range.clone()).unwrap_or(&[]) {
                entry.shown.set(true);
                entry.container().set_visible(true);
            }
        }
        self.refold();
    }

    /// Open every collapsed message, folded runs included — `O`.
    pub fn expand_all(&self) {
        let messages: Vec<MessageId> = self
            .imp()
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.message)
            .collect();
        for message in messages {
            self.expand(message);
        }
        for entry in self.imp().entries.borrow().iter() {
            entry.shown.set(true);
            entry.container().set_visible(true);
        }
        self.refold();
    }

    /// How many messages are showing their bodies.
    ///
    /// Asks the entries rather than counting readers: a message can be
    /// expanded before its body arrives, and "how much is open" is the
    /// question `Expand all` and the fold rules are about.
    pub fn expanded_count(&self) -> usize {
        self.imp()
            .entries
            .borrow()
            .iter()
            .filter(|entry| entry.expanded.get())
            .count()
    }

    /// The pane's header, for a test that wants to read what it says.
    pub fn header(&self) -> &Header {
        &self.imp().header
    }

    /// The conversation's own action bar.
    pub fn footer(&self) -> std::rc::Rc<crate::widgets::ActionBar> {
        std::rc::Rc::clone(&self.imp().footer)
    }

    /// What the folded-run dividers currently say, in stack order.
    /// Test-facing.
    pub fn divider_labels(&self) -> Vec<String> {
        self.imp()
            .dividers
            .borrow()
            .iter()
            .filter_map(|divider| {
                divider
                    .first_child()
                    .and_then(|child| child.next_sibling())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.label().to_string())
            })
            .collect()
    }

    /// How many messages the pane is holding.
    pub fn len(&self) -> usize {
        self.imp().entries.borrow().len()
    }

    /// Whether the pane is holding nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The current message: what both this pane and the drill-in column show,
    /// and what a per-message verb aims at.
    pub fn focused(&self) -> Option<MessageId> {
        self.imp().focused.get()
    }

    /// The current message's row.
    ///
    /// What the dwell timer marks read and what a reply is composed against,
    /// so the caller does not have to keep a second copy of the conversation
    /// and keep it in step.
    pub fn focused_row(&self) -> Option<Row> {
        let focused = self.imp().focused.get()?;
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == focused)
            .map(|entry| entry.row.clone())
    }

    /// Every message in the pane, in the order it is stacked.
    ///
    /// The drill-in column indexes this, and the two must agree on the order
    /// or jumping lands somewhere other than where it pointed.
    pub fn rows(&self) -> Vec<Row> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.row.clone())
            .collect()
    }

    /// Whether the focused message is drawn as focused.
    ///
    /// Asks the widget rather than the field, because "there is a focused
    /// message" and "you can see which one" are different claims and only the
    /// second one matters to somebody about to press reply.
    pub fn is_focus_drawn(&self) -> bool {
        let focused = self.imp().focused.get();
        self.imp()
            .entries
            .borrow()
            .iter()
            .any(|entry| Some(entry.message) == focused && entry.header.is_selected())
    }

    /// Whether `message` is showing its body.
    pub fn is_expanded(&self, message: MessageId) -> bool {
        self.imp()
            .entries
            .borrow()
            .iter()
            .any(|entry| entry.message == message && entry.expanded.get())
    }

    /// Make `message` the current one: expand it, draw it as focused, and
    /// scroll it into view.
    ///
    /// This is what the drill-in column's cursor calls. Landing on a
    /// collapsed header would be a dead end — you went there to read it — so
    /// jumping expands.
    pub fn focus_message(&self, message: MessageId) {
        if !self
            .imp()
            .entries
            .borrow()
            .iter()
            .any(|e| e.message == message)
        {
            return;
        }
        self.expand(message);
        self.imp().focused.set(Some(message));
        for entry in self.imp().entries.borrow().iter() {
            entry.header.set_selected(entry.message == message);
        }
        self.scroll_to(message);
        self.start_dwell(message);
        for handler in self.imp().on_focus.borrow().iter() {
            handler(message);
        }
    }

    /// Move focus to the next message in the stack — `J`.
    ///
    /// Stops at the end rather than wrapping: a conversation has a first and
    /// a last message, and wrapping from one to the other makes "am I at the
    /// end" a question you have to keep answering yourself. Steps *into* a
    /// folded run rather than over it — the run's messages are messages, and
    /// walking past five of them because they were drawn as one line would
    /// be the fold changing what the keyboard does.
    pub fn focus_next(&self) -> bool {
        self.step(1)
    }

    /// Move focus to the previous message in the stack — `K`.
    pub fn focus_previous(&self) -> bool {
        self.step(-1)
    }

    /// Fold or unfold the focused message — `space`.
    ///
    /// The only way to collapse the focused message: landing on one expands
    /// it ([`focus_message`](Self::focus_message)), so collapsed-and-focused
    /// is a state nothing else reaches.
    pub fn toggle_fold(&self) {
        let Some(focused) = self.focused() else {
            return;
        };
        if self.is_expanded(focused) {
            self.collapse(focused);
        } else {
            self.expand(focused);
        }
    }

    /// Where the focused message sits in the stack.
    pub fn focused_index(&self) -> Option<usize> {
        let focused = self.focused()?;
        self.imp()
            .entries
            .borrow()
            .iter()
            .position(|entry| entry.message == focused)
    }

    /// One step through the stack, in draw order.
    ///
    /// Answers whether it moved, so a caller can tell "there was nowhere to
    /// go" from "the pane is empty" — the first is a no-op the user will
    /// expect, the second means the key reached the wrong surface.
    fn step(&self, by: isize) -> bool {
        let entries = self.imp().entries.borrow();
        if entries.is_empty() {
            return false;
        }
        let Some(current) = self.focused_index() else {
            // Nothing focused: `J` starts at the beginning, `K` at the end.
            let landing = if by > 0 { 0 } else { entries.len() - 1 };
            let message = entries[landing].message;
            drop(entries);
            self.focus_message(message);
            return true;
        };
        let next = current as isize + by;
        if next < 0 || next as usize >= entries.len() {
            return false;
        }
        let message = entries[next as usize].message;
        drop(entries);
        self.focus_message(message);
        true
    }

    /// Give `message` a body, if it has not got one.
    ///
    /// Idempotent, and the only place the factory is called — which is what
    /// makes "one reader per expanded message, never per message" a property
    /// of this function rather than of every caller.
    pub fn expand(&self, message: MessageId) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let Some(entry) = entries.iter().find(|entry| entry.message == message) else {
            return;
        };
        if entry.expanded.get() {
            return;
        }
        entry.expanded.set(true);
        entry.actions.set_visible(true);
        if let Some(factory) = imp.factory.borrow().as_ref() {
            let reader = factory(message);
            let widget = reader.widget();
            widget.set_hexpand(true);
            entry.body.append(&widget);
            *entry.reader.borrow_mut() = Some(reader);
        }
        entry.body.set_visible(true);
    }

    /// The reader already built for `message`'s entry, if it has one.
    ///
    /// The conversation pane's answer to a body or a payload landing for an
    /// already-expanded entry (#739): `expand` only ever fills an *empty*
    /// body, so nothing re-fills a full one without a caller that can find
    /// the reader again and re-render into it — this is that seam. `None`
    /// for a collapsed entry (there is no reader yet; that is `expand`'s
    /// job) and for a message the pane is not holding at all.
    pub fn reader_for(&self, message: MessageId) -> Option<crate::reader::Reader> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == message && entry.expanded.get())
            .and_then(|entry| entry.reader.borrow().clone())
    }

    /// Collapse `message` back to its one-line header.
    pub fn collapse(&self, message: MessageId) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let Some(entry) = entries.iter().find(|entry| entry.message == message) else {
            return;
        };
        if !entry.expanded.get() {
            return;
        }
        entry.expanded.set(false);
        entry.body.set_visible(false);
        entry.actions.set_visible(false);
        // The body widget is kept rather than destroyed: a message collapsed
        // and reopened is the common gesture, and rebuilding a `WebKitWebView`
        // for it would make the cheap direction the expensive one.
    }

    /// Called when a message's reply button is used. `true` is reply-all.
    pub fn connect_reply(&self, handler: impl Fn(MessageId, bool) + 'static) {
        self.imp().on_reply.borrow_mut().push(Box::new(handler));
    }

    /// Called when a message's forward button is used.
    pub fn connect_forward(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_forward.borrow_mut().push(Box::new(handler));
    }

    /// Called when the current message changes, so the drill-in column can
    /// move its cursor to match.
    pub fn connect_focus_changed(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_focus.borrow_mut().push(Box::new(handler));
    }

    /// Called when a message has been focused long enough to have been read
    /// (#71).
    ///
    /// The same rule the list uses, one surface over: focus is what reading
    /// looks like here, and walking a conversation with the index passes over
    /// messages without reading them exactly as scrolling a mailbox does.
    /// Never fires for a conversation merely opened — the focused message has
    /// to be rested on.
    pub fn connect_dwelled(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_dwell.borrow_mut().push(Box::new(handler));
    }

    /// Re-cap every message's action bar from the live keymap.
    ///
    /// Called by `Window::apply_keymap` alongside every other surface that
    /// shows a key: a `[keys]` rebind has to reach the caps in the stack the
    /// same moment it reaches the keyboard, or the pane advertises a key
    /// that now runs something else.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        for entry in self.imp().entries.borrow().iter() {
            entry.actions.set_keymap(keymap);
        }
        self.imp().header.set_keymap(keymap);
        self.imp().footer.set_keymap(keymap);
    }

    /// Shorten the dwell for a test that cannot wait a second.
    pub fn set_dwell_delay(&self, delay: std::time::Duration) {
        self.imp().dwell_delay.set(delay);
    }

    /// Stop any dwell in flight.
    ///
    /// Cancelled rather than dropped: a `glib` timeout whose handle goes away
    /// still fires, and this one would mark a message read after the pane had
    /// moved on from it.
    pub fn cancel_dwell(&self) {
        if let Some(source) = self.imp().dwell.borrow_mut().take() {
            source.remove();
        }
    }

    fn start_dwell(&self, message: MessageId) {
        self.cancel_dwell();
        let view = self.clone();
        let source = glib::timeout_add_local_once(self.imp().dwell_delay.get(), move || {
            view.imp().dwell.borrow_mut().take();
            // Named explicitly rather than read back off the focus: focus may
            // have moved between the timer firing and this running, and the
            // message that was read is the one the clock was started for.
            for handler in view.imp().on_dwell.borrow().iter() {
                handler(message);
            }
        });
        *self.imp().dwell.borrow_mut() = Some(source);
    }

    /// Press a message's reply button without a pointer.
    pub fn test_click_reply(&self, message: MessageId) {
        for handler in self.imp().on_reply.borrow().iter() {
            handler(message, false);
        }
    }

    /// Press a message's forward button without a pointer.
    pub fn test_click_forward(&self, message: MessageId) {
        for handler in self.imp().on_forward.borrow().iter() {
            handler(message);
        }
    }

    /// The widget the reader factory built for `message`, if it has been
    /// expanded — what a test downcasts to check what is actually on
    /// screen inside an entry, rather than only what the pane's own state
    /// says (#487).
    #[doc(hidden)]
    pub fn test_expanded_widget(&self, message: MessageId) -> Option<gtk::Widget> {
        self.imp()
            .entries
            .borrow()
            .iter()
            .find(|entry| entry.message == message)
            .and_then(|entry| entry.body.first_child())
    }

    // -- internals ---------------------------------------------------------

    fn scroll_to(&self, message: MessageId) {
        let imp = self.imp();
        let entries = imp.entries.borrow();
        let Some(entry) = entries.iter().find(|entry| entry.message == message) else {
            return;
        };
        // No animation: the motion budget says a jump is instant, and this is
        // the same swap the drill-in itself makes.
        let container = entry.container();
        let Some(bounds) = container.compute_bounds(&imp.stack) else {
            return;
        };
        imp.scroller.vadjustment().set_value(bounds.y() as f64);
    }

    fn build_entry(&self, row: &Row, index: u32) -> imp::Entry {
        let header = crate::thread_row::ThreadRowView::new();
        header.set_row(Some(row.clone()), index);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_visible(false);

        // Reply, reply-all and forward only. Every other verb is the
        // conversation's, in this pane exactly as in the list (ADR 0015 Q4),
        // and a delete button on every message in a stack is how people
        // delete the wrong one.
        //
        // The same `ActionBar` the reading pane's own bar is (#1002), so
        // these three carry their keys — they were three bare buttons with
        // no caps at all, in a pane whose whole point is that `e` acts on
        // whichever message you are looking at.
        let message = row.id;
        let actions = crate::widgets::ActionBar::new(&MESSAGE_ACTIONS, "conversation-actions");
        actions.set_visible(false);
        let view = self.clone();
        actions.connect_command(move |command| {
            let kind = match command.id() {
                postio_core::CommandId::Reply => ReplyKind::Reply,
                postio_core::CommandId::ReplyAll => ReplyKind::ReplyAll,
                postio_core::CommandId::Forward => ReplyKind::Forward,
                _ => return,
            };
            view.emit_action(message, kind);
        });

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("conversation-entry");
        container.append(&header);
        container.append(&body);
        // Under the message, not under its header. Above the body they read
        // as belonging to whatever comes next -- which for a stack of
        // messages is somebody else's mail.
        container.append(&actions.widget());

        // A click anywhere on the header makes that message current, which is
        // the mouse's half of what the column's cursor does.
        let gesture = gtk::GestureClick::new();
        let view = self.clone();
        gesture.connect_released(move |_, _, _, _| view.focus_message(message));
        header.add_controller(gesture);

        imp::Entry {
            message,
            row: row.clone(),
            header,
            body,
            reader: std::cell::RefCell::new(None),
            actions,
            expanded: std::cell::Cell::new(false),
            shown: std::cell::Cell::new(false),
            container,
        }
    }

    fn emit_action(&self, message: MessageId, kind: ReplyKind) {
        match kind {
            ReplyKind::Reply => {
                for handler in self.imp().on_reply.borrow().iter() {
                    handler(message, false);
                }
            }
            ReplyKind::ReplyAll => {
                for handler in self.imp().on_reply.borrow().iter() {
                    handler(message, true);
                }
            }
            ReplyKind::Forward => {
                for handler in self.imp().on_forward.borrow().iter() {
                    handler(message);
                }
            }
        }
    }
}

/// Which per-message verb a button carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}
