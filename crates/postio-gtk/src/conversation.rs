//! The conversation pane: every message of a thread, stacked (ADR 0015 Q4).
//!
//! The reading pane used to show one message. Opening a conversation now
//! fills it with all of them, oldest first, read ones collapsed to a one-line
//! header and the rest expanded onto the reader Postio already has.
//!
//! # The column is an index; this is the conversation
//!
//! `crate::thread::ThreadView` still lists a line per message in the pane the
//! message list occupies, and the two are not the same conversation drawn
//! twice: the column is a **table of contents**. Moving its cursor scrolls
//! this pane and expands what it lands on. There is one current message and
//! both surfaces show it.
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

type MessageHandler = Box<dyn Fn(MessageId)>;
type ReplyHandler = Box<dyn Fn(MessageId, bool)>;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct ConversationView {
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
        pub actions: gtk::Box,
        pub expanded: Cell<bool>,
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
            self.scroller.set_parent(&*view);
        }

        fn dispose(&self) {
            self.scroller.unparent();
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
        Self::default()
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
        imp.focused.set(None);
        self.cancel_dwell();

        let focus = opening_focus(&messages);
        let expanded = match focus {
            Some(focus) => expanded_on_open(&messages, focus, EAGER_EXPANSION_CAP),
            None => Vec::new(),
        };

        for (index, row) in messages.iter().enumerate() {
            // Numbered from one, matching the column exactly: the two are
            // the same conversation and a reader comparing them must not have
            // to add one in their head.
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
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.add_css_class("conversation-actions");
        actions.set_visible(false);
        let message = row.id;
        for (label, handler) in [
            ("Reply", ReplyKind::Reply),
            ("Reply all", ReplyKind::ReplyAll),
            ("Forward", ReplyKind::Forward),
        ] {
            let button = gtk::Button::with_label(label);
            button.add_css_class("flat");
            let view = self.clone();
            button.connect_clicked(move |_| view.emit_action(message, handler));
            actions.append(&button);
        }

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("conversation-entry");
        container.append(&header);
        container.append(&body);
        // Under the message, not under its header. Above the body they read
        // as belonging to whatever comes next -- which for a stack of
        // messages is somebody else's mail.
        container.append(&actions);

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
