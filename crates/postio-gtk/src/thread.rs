//! Thread drill-in: the list column becomes the thread, and `Esc` brings it
//! back.
//!
//! Canvas 3a. `t` on a row replaces the message list with that row's thread —
//! one compact line per message, numbered, with the focused one showing in the
//! reading pane beside it. `Esc` returns.
//!
//! # Why the list is hidden and not rebuilt
//!
//! The acceptance criterion is that the round trip restores the exact prior
//! scroll position and selection. The cheapest way to restore state is not to
//! disturb it: the drill-in *hides* [`crate::list_view::MessageListView`] and
//! shows this in the same pane, so the model, the scroll adjustment, the
//! cursor and the selection are never touched and there is nothing to save or
//! put back. A design that rebuilt the list on the way out would have to
//! reproduce all four, and would be wrong the first time any of them grew a
//! field.
//!
//! It is also what makes the switch instant. `Shell` already swaps its panes
//! with nothing but `set_visible` for the same reason (CLAUDE.md's motion
//! budget: pane switches use *no* transition), and this is the same move one
//! level down.
//!
//! # Where the messages come from
//!
//! From the rows the list already holds. The list is a windowed model over the
//! mailbox, so it knows every message it has paged in — and in a per-message
//! list, the rows sharing a `ThreadId` *are* the thread as this folder sees
//! it. That needs no new read path, which is why `t` works today rather than
//! after something wires a thread query.
//!
//! It is also, honestly, not always the whole thread: a message filed in
//! Archive is not in this folder's model, and a page the list has not reached
//! is not resident either. [`Row::thread_count`] says how many there really
//! are, so when the two disagree the header says which is which rather than
//! quietly showing a short thread. `postio-6p1` tracks reading the rest.
//!
//! [`Row::thread_count`]: crate::list::Row::thread_count

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib, pango};
use postio_model::ids::{MessageId, ThreadId};

use crate::list::Row;

/// The keys the column offers, drawn at its foot.
///
/// The canvas draws `Esc back` here as well as in the header line above. Only
/// once, here: the header already says `Esc back to Inbox`, which is the same
/// key and names where it goes, and a 404px column has room for the two view
/// toggles beside this or for a hint it has already given, not both.
const THREAD_KEYS: &str = "j/k in thread · A archive thread";

/// What the column says when a filter has hidden everything.
const NOTHING_UNREAD: &str = "Every message here has been read.";

/// How a thread orders its messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    /// Oldest first — how a conversation was actually had, and what the
    /// canvas numbers `1..n` down the column.
    #[default]
    Oldest,
    /// Newest first, matching the message list the drill-in came from.
    Newest,
}

impl Order {
    /// The other one.
    pub fn toggled(self) -> Order {
        match self {
            Order::Oldest => Order::Newest,
            Order::Newest => Order::Oldest,
        }
    }

    /// The whole phrase — what a screen reader hears, and what the tooltip
    /// promises the other one would do.
    pub const fn label(self) -> &'static str {
        match self {
            Order::Oldest => "Oldest first",
            Order::Newest => "Newest first",
        }
    }

    /// The button face. One word, because the column it sits in is 404px
    /// wide and the key hints beside it are not negotiable.
    pub const fn short(self) -> &'static str {
        match self {
            Order::Oldest => "Oldest",
            Order::Newest => "Newest",
        }
    }
}

/// The rows a thread shows, given what is in it and how it is being viewed.
///
/// Pure, and tested without a display: the ordering and the filtering are the
/// part worth being sure about, and they have nothing to do with GTK.
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

/// How many distinct people are in a thread.
///
/// By address, folded: one correspondent who has changed their display name
/// mid-thread is still one person, and the canvas' `4 people` is a count of
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

/// The line under the subject: how much of the thread this is, who is in it,
/// and the way out.
///
/// `total` is what [`Row::thread_count`](crate::list::Row::thread_count)
/// claims the thread holds. When it is larger than what is on screen the line
/// says so — a column that showed three of six messages and called it the
/// thread would be lying about the one thing the user came here to see.
pub fn summary(shown: usize, total: u32, people: usize, back_to: Option<&str>) -> String {
    let messages = match (shown, total as usize) {
        (1, whole) if whole <= 1 => "1 message".to_string(),
        (shown, whole) if shown >= whole => format!("{shown} messages"),
        (shown, whole) => format!("{shown} of {whole} messages here"),
    };
    let people = match people {
        0 => String::new(),
        1 => " · 1 person".to_string(),
        many => format!(" · {many} people"),
    };
    let back = match back_to {
        Some(name) if !name.is_empty() => format!(" · Esc back to {name}"),
        _ => " · Esc back".to_string(),
    };
    format!("{messages}{people}{back}")
}

type ActivatedHandler = Box<dyn Fn(MessageId)>;

mod imp {
    use super::*;

    pub struct ThreadView {
        pub(super) subject: gtk::Label,
        pub(super) meta: gtk::Label,
        pub(super) back: gtk::Button,
        pub(super) unread_toggle: gtk::ToggleButton,
        pub(super) order_toggle: gtk::Button,
        /// The messages on screen, as a model — so the column recycles row
        /// widgets instead of building one per message. See
        /// [`super::ThreadView::rebuild`].
        pub(super) store: gio::ListStore,
        pub(super) cursor: gtk::SingleSelection,
        pub(super) rows: gtk::ListView,
        pub(super) scroller: gtk::ScrolledWindow,
        pub(super) empty: gtk::Label,
        /// Every message of the thread this column knows about, unfiltered
        /// and unsorted — [`super::arrange`] derives what is drawn.
        pub(super) all: RefCell<Vec<Row>>,
        /// What is drawn, in the order it is drawn.
        pub(super) shown: RefCell<Vec<Row>>,
        pub(super) thread: Cell<Option<ThreadId>>,
        pub(super) total: Cell<u32>,
        pub(super) back_to: RefCell<Option<String>>,
        pub(super) order: Cell<Order>,
        pub(super) unread_only: Cell<bool>,
        /// Set while the column is moving its own selection, so restoring a
        /// cursor does not read as the user picking a row.
        pub(super) echoing: Cell<bool>,
        pub(super) on_activated: RefCell<Vec<ActivatedHandler>>,
        pub(super) on_back: RefCell<Vec<Box<dyn Fn()>>>,
        /// The clock every row in one rebuild formats its timestamp against.
        ///
        /// Read once per rebuild rather than once per row: `Local::now()`
        /// resolves the timezone each call, and a long thread paid for it
        /// two hundred times. It is also the more correct answer — every row
        /// in one draw should agree about what "now" is, or a thread redrawn
        /// across a second boundary can show two different relative days.
        pub(super) now: Rc<Cell<chrono::DateTime<chrono::Local>>>,
    }

    impl Default for ThreadView {
        fn default() -> Self {
            let store = gio::ListStore::new::<crate::list::MessageRow>();
            let cursor = gtk::SingleSelection::new(Some(store.clone()));
            ThreadView {
                subject: gtk::Label::default(),
                meta: gtk::Label::default(),
                back: gtk::Button::default(),
                unread_toggle: gtk::ToggleButton::default(),
                order_toggle: gtk::Button::default(),
                rows: gtk::ListView::new(Some(cursor.clone()), None::<gtk::ListItemFactory>),
                store,
                cursor,
                scroller: gtk::ScrolledWindow::default(),
                empty: gtk::Label::default(),
                all: RefCell::default(),
                shown: RefCell::default(),
                thread: Cell::default(),
                total: Cell::default(),
                back_to: RefCell::default(),
                order: Cell::default(),
                unread_only: Cell::default(),
                echoing: Cell::default(),
                on_activated: RefCell::default(),
                on_back: RefCell::default(),
                now: Rc::new(Cell::new(chrono::Local::now())),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThreadView {
        const NAME: &'static str = "PostioThreadView";
        type Type = super::ThreadView;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for ThreadView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for ThreadView {}
    impl BinImpl for ThreadView {}
}

glib::wrapper! {
    /// The thread, where the message list was.
    pub struct ThreadView(ObjectSubclass<imp::ThreadView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ThreadView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ThreadView {
    /// An empty column.
    pub fn new() -> Self {
        Self::default()
    }

    /// The thread on screen, if any.
    pub fn thread(&self) -> Option<ThreadId> {
        self.imp().thread.get()
    }

    /// Show `thread`.
    ///
    /// `total` is what the list row claimed the thread holds, and `back_to`
    /// the folder `Esc` returns to. `rows` may be a subset of the thread —
    /// see the module docs — and the header says so when it is.
    pub fn open(
        &self,
        thread: ThreadId,
        subject: Option<&str>,
        rows: Vec<Row>,
        total: u32,
        back_to: Option<&str>,
    ) {
        let imp = self.imp();
        imp.thread.set(Some(thread));
        imp.total.set(total.max(rows.len() as u32));
        *imp.back_to.borrow_mut() = back_to.map(str::to_owned);
        *imp.all.borrow_mut() = rows;

        let subject = subject
            .map(str::trim)
            .filter(|subject| !subject.is_empty())
            .unwrap_or("(no subject)");
        imp.subject.set_text(subject);
        imp.subject.set_tooltip_text(Some(subject));

        self.rebuild();
        // The newest message is what the drill-in is usually about, and it is
        // where the canvas draws the selection.
        self.last_row();
    }

    /// Empty the column.
    pub fn close(&self) {
        let imp = self.imp();
        imp.thread.set(None);
        imp.all.borrow_mut().clear();
        imp.shown.borrow_mut().clear();
        imp.unread_only.set(false);
        imp.unread_toggle.set_active(false);
        imp.order.set(Order::default());
        self.rebuild();
    }

    /// The messages on screen, in the order they are drawn.
    pub fn rows(&self) -> Vec<Row> {
        self.imp().shown.borrow().clone()
    }

    /// The message the keyboard is on.
    pub fn cursor(&self) -> Option<MessageId> {
        let index = self.imp().cursor.selected();
        if index == gtk::INVALID_LIST_POSITION {
            return None;
        }
        self.imp()
            .shown
            .borrow()
            .get(index as usize)
            .map(|row| row.id)
    }

    /// Whether the column is hiding what has been read.
    pub fn unread_only(&self) -> bool {
        self.imp().unread_only.get()
    }

    /// Hide, or stop hiding, what has been read.
    pub fn set_unread_only(&self, unread_only: bool) {
        let imp = self.imp();
        if imp.unread_only.replace(unread_only) == unread_only {
            return;
        }
        imp.unread_toggle.set_active(unread_only);
        self.rebuild();
    }

    /// Which way round the messages are.
    pub fn order(&self) -> Order {
        self.imp().order.get()
    }

    /// Put the messages the other way round.
    ///
    /// Keeps the keyboard on the same *message* rather than the same row
    /// index — reversing the column under the cursor and leaving it pointing
    /// at whatever moved into that slot is the sort of thing that makes a
    /// list feel untrustworthy.
    pub fn set_order(&self, order: Order) {
        let imp = self.imp();
        if imp.order.replace(order) == order {
            return;
        }
        let cursor = self.cursor();
        self.rebuild();
        if let Some(cursor) = cursor {
            self.focus_message(cursor);
        }
    }

    /// Move the keyboard one message down the column.
    pub fn next_row(&self) {
        self.move_cursor(1);
    }

    /// Move the keyboard one message up the column.
    pub fn prev_row(&self) {
        self.move_cursor(-1);
    }

    /// The first message in the column.
    pub fn first_row(&self) {
        self.select_index(0);
    }

    /// The last message in the column.
    pub fn last_row(&self) {
        let last = self.imp().shown.borrow().len();
        self.select_index(last.saturating_sub(1) as i32);
    }

    /// Put the keyboard on `message`, if it is on screen.
    pub fn focus_message(&self, message: MessageId) {
        let index = self
            .imp()
            .shown
            .borrow()
            .iter()
            .position(|row| row.id == message);
        if let Some(index) = index {
            self.select_index(index as i32);
        }
    }

    /// Put the keyboard in the column itself, so the focus ring is visible
    /// and the cursor row is where the eye goes.
    pub fn focus_rows(&self) {
        let imp = self.imp();
        let selected = imp.cursor.selected();
        if selected == gtk::INVALID_LIST_POSITION {
            imp.rows.grab_focus();
            return;
        }
        imp.rows
            .scroll_to(selected, gtk::ListScrollFlags::FOCUS, None);
    }

    /// Called when the cursor lands on a message — which is what shows it in
    /// the reading pane.
    pub fn connect_activated(&self, handler: impl Fn(MessageId) + 'static) {
        self.imp().on_activated.borrow_mut().push(Box::new(handler));
    }

    /// Called when the column's own back button is used.
    ///
    /// The button only emits; the window runs the registry's `Back`, so the
    /// mouse and `Esc` are one path rather than two.
    pub fn connect_back(&self, handler: impl Fn() + 'static) {
        self.imp().on_back.borrow_mut().push(Box::new(handler));
    }

    // -- internals ---------------------------------------------------------

    fn move_cursor(&self, delta: i32) {
        let count = self.imp().shown.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let selected = self.imp().cursor.selected();
        let current = if selected == gtk::INVALID_LIST_POSITION {
            if delta >= 0 { -1 } else { count }
        } else {
            selected as i32
        };
        self.select_index((current + delta).clamp(0, count - 1));
    }

    fn select_index(&self, index: i32) {
        let imp = self.imp();
        let count = imp.shown.borrow().len() as i32;
        if index < 0 || index >= count {
            return;
        }
        let index = index as u32;
        imp.cursor.set_selected(index);
        // Keep the cursor row on screen, and put the keyboard on it. No
        // animation: the motion budget applies to a column as much as to a
        // pane, and `scroll_to` here jumps.
        imp.rows.scroll_to(index, gtk::ListScrollFlags::FOCUS, None);
        self.announce();
    }

    /// Tell whoever is listening which message the cursor is on.
    fn announce(&self) {
        if self.imp().echoing.get() {
            return;
        }
        let Some(message) = self.cursor() else { return };
        for handler in self.imp().on_activated.borrow().iter() {
            handler(message);
        }
    }

    /// Redraw the column from `all`, `order` and `unread_only`.
    ///
    /// Rebuilt whole rather than diffed. A thread is bounded by the
    /// conversation — a few hundred messages at the very worst, against a
    /// mailbox's tens of thousands — so there is nothing to gain from an
    /// incremental redraw and a stale row to lose by it. The one thing that
    /// would not be affordable, materialising a *mailbox*, is exactly what
    /// this is not.
    fn rebuild(&self) {
        let imp = self.imp();
        let shown = arrange(&imp.all.borrow(), imp.order.get(), imp.unread_only.get());
        let empty = shown.is_empty();

        // Before the splice, not after. A `GtkListView` whose `ScrolledWindow`
        // is hidden has nothing to scroll inside, so it instantiates a row
        // widget for *every* item in the model rather than a screenful —
        // measured at 200 setups and 200 binds for a 200-message thread,
        // blowing the 16ms an interaction gets. With the scroller showing
        // first it builds the fifteen rows that fit and recycles them.
        imp.scroller.set_visible(!empty);
        imp.empty.set_visible(empty);
        imp.empty.set_text(NOTHING_UNREAD);

        // One splice, and the row widgets are recycled by the factory: a
        // 200-message thread costs the same handful of widgets a 6-message
        // one does.
        imp.now.set(chrono::Local::now());
        imp.echoing.set(true);
        let items: Vec<crate::list::MessageRow> = shown
            .iter()
            .map(|row| crate::list::MessageRow::new(row.clone()))
            .collect();
        imp.store.splice(0, imp.store.n_items(), &items);
        imp.echoing.set(false);

        let people = people(&shown);
        *imp.shown.borrow_mut() = shown;

        imp.meta.set_text(&summary(
            imp.shown.borrow().len(),
            imp.total.get(),
            people,
            imp.back_to.borrow().as_deref(),
        ));
        let order = imp.order.get();
        imp.order_toggle.set_label(order.short());
        imp.order_toggle
            .set_tooltip_text(Some(order.toggled().label()));
        imp.order_toggle
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "{}. Activate for {}",
                order.label(),
                order.toggled().label().to_lowercase()
            ))]);
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-thread");
        self.set_visible(false);

        imp.back.set_icon_name("go-previous-symbolic");
        imp.back.add_css_class("flat");
        imp.back.add_css_class("postio-thread-back");
        imp.back
            .update_property(&[gtk::accessible::Property::Label("Back to the message list")]);
        imp.back.set_tooltip_text(Some("Back to the message list"));
        imp.back.connect_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| {
                for handler in view.imp().on_back.borrow().iter() {
                    handler();
                }
            }
        ));

        imp.subject.add_css_class("postio-thread-subject");
        imp.subject.set_xalign(0.0);
        imp.subject.set_ellipsize(pango::EllipsizeMode::End);
        imp.subject
            .set_accessible_role(gtk::AccessibleRole::Caption);

        imp.meta.add_css_class("postio-thread-meta");
        imp.meta.set_xalign(0.0);
        imp.meta.set_ellipsize(pango::EllipsizeMode::End);

        let title = gtk::Box::new(gtk::Orientation::Vertical, 0);
        title.set_hexpand(true);
        title.append(&imp.subject);
        title.append(&imp.meta);

        // Real buttons rather than key hints. These two are view options that
        // the command registry does not carry, and drawing `u` beside a
        // filter that `u` does not reach would teach the wrong key — the one
        // thing the whole hint system depends on not doing. As buttons they
        // are still keyboard-operable, by Tab and Space, and `postio-yzc`
        // tracks giving them verbs of their own.
        imp.unread_toggle.set_label("Unread");
        imp.unread_toggle
            .update_property(&[gtk::accessible::Property::Label(
                "Show only what has not been read",
            )]);
        imp.unread_toggle.add_css_class("flat");
        imp.unread_toggle.add_css_class("postio-thread-toggle");
        imp.unread_toggle
            .set_tooltip_text(Some("Show only what has not been read"));
        imp.unread_toggle.connect_toggled(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |toggle| view.set_unread_only(toggle.is_active())
        ));

        imp.order_toggle.set_label(Order::default().short());
        imp.order_toggle.add_css_class("flat");
        imp.order_toggle.add_css_class("postio-thread-toggle");
        imp.order_toggle.connect_clicked(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.set_order(view.order().toggled())
        ));

        // The header is the canvas' header and nothing else: a back button,
        // the subject, and the line saying how much of the thread this is.
        // The two view toggles went to the foot instead — at 404px they ate
        // enough of the header to ellipsize `Esc back to Inbox`, and a way
        // out that does not say where it goes is the one thing that line is
        // for.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("postio-thread-header");
        header.append(&imp.back);
        header.append(&title);

        imp.rows.set_factory(Some(&row_factory(imp.now.clone())));
        imp.rows.add_css_class("postio-thread-rows");
        imp.rows.set_show_separators(false);
        imp.rows.set_single_click_activate(false);
        imp.rows.set_vexpand(true);
        imp.rows
            .update_property(&[gtk::accessible::Property::Label("Messages in this thread")]);
        imp.cursor.set_autoselect(false);
        imp.cursor.connect_selected_notify(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_| view.announce()
        ));
        // A click and `Ret` mean the same thing here — the message is already
        // shown when the cursor lands on it, so activating is how a mouse
        // says "this one" without also having to move a keyboard cursor.
        imp.rows.connect_activate(glib::clone!(
            #[weak(rename_to = view)]
            self,
            move |_, _| view.announce()
        ));

        imp.scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.scroller.set_vexpand(true);
        imp.scroller.set_child(Some(&imp.rows));

        imp.empty.add_css_class("postio-thread-empty");
        imp.empty.set_xalign(0.0);
        imp.empty.set_wrap(true);
        imp.empty.set_vexpand(true);
        imp.empty.set_valign(gtk::Align::Start);
        imp.empty.set_visible(false);

        let keys = gtk::Label::new(Some(THREAD_KEYS));
        keys.add_css_class("postio-thread-keys");
        keys.set_xalign(0.0);
        keys.set_hexpand(true);
        keys.set_ellipsize(pango::EllipsizeMode::End);
        keys.set_accessible_role(gtk::AccessibleRole::Presentation);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        footer.add_css_class("postio-thread-footer");
        footer.append(&keys);
        footer.append(&imp.unread_toggle);
        footer.append(&imp.order_toggle);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&imp.scroller);
        column.append(&imp.empty);
        column.append(&footer);
        self.set_child(Some(&column));

        self.rebuild();
    }
}

/// Builds and rebinds one row.
///
/// One widget per row, not four labels in a box. `postio-p44` measured the
/// difference on a `GtkListView`'s read-ahead window: 18.3ms against 6.8ms,
/// which is the difference between sitting inside a 16ms frame and not. See
/// [`crate::thread_row`].
fn row_factory(now: Rc<Cell<chrono::DateTime<chrono::Local>>>) -> gtk::SignalListItemFactory {
    let _ = now;
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        item.set_child(Some(&crate::thread_row::ThreadRowView::new()));
        item.set_activatable(true);
        // The cursor lives on the list item and its state flag does not reach
        // the child, so it is handed down explicitly — and kept in step,
        // because moving the cursor off this row is a change to this row.
        item.connect_selected_notify(|item| {
            if let Some(view) = item
                .child()
                .and_downcast::<crate::thread_row::ThreadRowView>()
            {
                view.set_selected(item.is_selected());
            }
        });
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(view) = item
            .child()
            .and_downcast::<crate::thread_row::ThreadRowView>()
        else {
            return;
        };
        let row = item
            .item()
            .and_downcast::<crate::list::MessageRow>()
            .and_then(|item| item.row());
        // The canvas numbers the column `1..n`, which is a fact about what is
        // drawn rather than about the message — so it comes off the list item
        // and follows the order toggle for free.
        view.set_row(row, item.position() + 1);
        view.set_selected(item.is_selected());
        item.set_accessible_label(&view.spoken());
    });
    factory.connect_unbind(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_accessible_label("");
            if let Some(view) = item
                .child()
                .and_downcast::<crate::thread_row::ThreadRowView>()
            {
                view.set_row(None, 0);
            }
        }
    });
    factory
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use postio_model::EmailAddress;

    fn at(hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 23, hour, 0, 0).unwrap()
    }

    fn message(id: i64, name: &str, address: &str, hour: u32, seen: bool) -> Row {
        Row {
            id: MessageId::new(id),
            thread: Some(ThreadId::new(1)),
            from: Some(EmailAddress::new(Some(name), address)),
            subject: Some("Re: index rebuild".to_owned()),
            preview: None,
            received_at: at(hour),
            seen,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 3,
        }
    }

    fn thread() -> Vec<Row> {
        vec![
            message(3, "Nadia Okafor", "nadia@example.org", 11, true),
            message(1, "Diogo Ferreira", "diogo@example.org", 9, true),
            message(2, "Lena Tomlin", "lena@example.com", 10, false),
        ]
    }

    // -- arranging ---------------------------------------------------------

    #[test]
    fn a_thread_reads_oldest_first_by_default() {
        let shown = arrange(&thread(), Order::default(), false);
        let ids: Vec<i64> = shown.iter().map(|row| row.id.get()).collect();

        assert_eq!(ids, [1, 2, 3], "the conversation, in the order it happened");
    }

    #[test]
    fn the_order_toggle_turns_the_column_round() {
        let shown = arrange(&thread(), Order::Newest, false);
        let ids: Vec<i64> = shown.iter().map(|row| row.id.get()).collect();

        assert_eq!(ids, [3, 2, 1]);
        assert_eq!(Order::Oldest.toggled(), Order::Newest);
        assert_eq!(Order::Newest.toggled(), Order::Oldest);
    }

    #[test]
    fn two_messages_in_the_same_second_keep_a_stable_order() {
        let mut rows = thread();
        for row in &mut rows {
            row.received_at = at(9);
        }
        let once = arrange(&rows, Order::Oldest, false);
        rows.reverse();
        let again = arrange(&rows, Order::Oldest, false);

        assert_eq!(
            once.iter().map(|row| row.id).collect::<Vec<_>>(),
            again.iter().map(|row| row.id).collect::<Vec<_>>(),
            "a redraw must not shuffle messages that share a timestamp"
        );
    }

    #[test]
    fn the_unread_filter_keeps_only_what_has_not_been_read() {
        let shown = arrange(&thread(), Order::Oldest, true);
        let ids: Vec<i64> = shown.iter().map(|row| row.id.get()).collect();

        assert_eq!(ids, [2]);
    }

    #[test]
    fn filtering_a_fully_read_thread_leaves_nothing_rather_than_everything() {
        let mut rows = thread();
        for row in &mut rows {
            row.seen = true;
        }
        assert!(arrange(&rows, Order::Oldest, true).is_empty());
    }

    // -- counting people ---------------------------------------------------

    #[test]
    fn people_are_counted_by_address_not_by_header() {
        let mut rows = thread();
        // The same correspondent, with the name spelled differently.
        rows.push(message(4, "L. Tomlin", "LENA@example.com", 12, true));

        assert_eq!(people(&rows), 3, "three correspondents, four messages");
    }

    #[test]
    fn a_thread_with_no_senders_has_no_people() {
        let mut rows = thread();
        for row in &mut rows {
            row.from = None;
        }
        assert_eq!(people(&rows), 0);
    }

    // -- the summary line --------------------------------------------------

    #[test]
    fn the_summary_reads_the_way_the_canvas_writes_it() {
        assert_eq!(
            summary(6, 6, 4, Some("Inbox")),
            "6 messages · 4 people · Esc back to Inbox"
        );
    }

    #[test]
    fn a_thread_the_column_cannot_see_all_of_says_so() {
        assert_eq!(
            summary(3, 6, 2, Some("Inbox")),
            "3 of 6 messages here · 2 people · Esc back to Inbox",
            "calling three of six messages `the thread` would be a lie"
        );
    }

    #[test]
    fn one_of_anything_is_not_plural() {
        assert_eq!(
            summary(1, 1, 1, Some("Inbox")),
            "1 message · 1 person · Esc back to Inbox"
        );
    }

    #[test]
    fn a_summary_with_nowhere_named_still_names_the_way_out() {
        assert_eq!(summary(2, 2, 1, None), "2 messages · 1 person · Esc back");
        assert_eq!(
            summary(2, 2, 1, Some("")),
            "2 messages · 1 person · Esc back"
        );
    }

    #[test]
    fn a_stale_total_never_makes_the_count_read_backwards() {
        // The list row's badge can lag what the column actually holds.
        assert_eq!(
            summary(6, 2, 3, Some("Inbox")),
            "6 messages · 3 people · Esc back to Inbox"
        );
    }
}
