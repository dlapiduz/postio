//! The message list pane: canvas 1b's header, and the rows under it.
//!
//! This is the shallow half of the list — the deep half is [`crate::list`]'s
//! windowed model and [`crate::row`]'s hand-drawn row. What lives here is the
//! `GtkListView` that puts the two together, and the header line the canvas
//! draws above them: the folder, its unread count, and the sort order.
//!
//! # Why a `GtkListView` and not a `GtkListBox`
//!
//! `GtkListBox` materialises a widget per item, which is the one thing
//! docs/PRODUCT.md §18 forbids: a 100,000-message folder would cost 100,000 widgets.
//! `GtkListView` recycles a screenful, asks the model only for the rows it is
//! about to draw, and hands each one to the same handful of
//! [`MessageRowView`]s over and over. That is what makes the windowed model
//! worth having.
//!
//! # The accessible row is the list item, not the widget inside it
//!
//! `GtkListItemWidget` is what takes focus, what carries the `ListItem` role
//! and what a screen reader navigates the list by. [`MessageRowView`] paints
//! its own text rather than holding labels, so GTK has nothing to compute a
//! name from and every row would otherwise announce as an unnamed list item.
//! The sentence is handed over on bind, through
//! `GtkListItem::set_accessible_label` — which is the API for exactly this,
//! and not the same thing as pushing an accessible property onto the item
//! widget: doing *that* from inside `bind` segfaults, because GTK is
//! part-way through its own item bookkeeping at the time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use postio_config::Density;
use postio_core::state::Selection;
use postio_core::{Command, CommandId, Keymap, MessageTarget};
use postio_model::MessageId;

use crate::list::{MessageList, MessageRow};
use crate::row::MessageRowView;
use crate::selection::{self, SelectionState};

/// What to call when a row is activated — `Enter`, or a double click.
type ActivateHandler = Box<dyn Fn(crate::list::Row)>;

/// What to call when the cursor lands on a row — `j`, `k`, a click, `G`.
///
/// Separate from [`ActivateHandler`] because in this pane the cursor, the
/// selection and an activation are three different facts. The reading pane
/// follows this one: moving the cursor changes what the user is looking at,
/// and nothing should wait for Return. See issue #70.
type CursorHandler = Box<dyn Fn(crate::list::Row)>;

/// What to call when the mouse runs a command.
///
/// A whole [`Command`] rather than a [`CommandId`], because the mouse can be
/// more specific than a keystroke: `a` archives the selection, but the
/// archive glyph on a row archives *that row*, and the target is the only
/// place that difference lives.
type CommandHandler = Box<dyn Fn(Command)>;

/// The verbs the bulk bar carries, in the order they appear.
///
/// Three fit the canvas' 404px list without crowding, and these are the three
/// triage is made of. Everything else a selection can do stays one `Ctrl+K`
/// away — a bar that grew a button per command would be a toolbar, which is
/// the thing this app is not.
const BULK_ACTIONS: [(CommandId, &str, &str); 3] = [
    (CommandId::Archive, "Archive", "a"),
    (CommandId::Delete, "Delete", "d"),
    (CommandId::Move, "Move", "m"),
];

mod imp {
    use super::*;

    pub struct MessageListView {
        pub(super) title: gtk::Label,
        pub(super) meta: gtk::Label,
        pub(super) sort: gtk::Label,
        /// "12 selected", and the bar of verbs beside it.
        pub(super) count: gtk::Label,
        pub(super) bulk: gtk::Box,
        pub(super) model: MessageList,
        /// The **cursor**: which row the keyboard is on. GTK calls this a
        /// selection; this pane does not — see [`crate::selection`].
        pub(super) cursor: gtk::SingleSelection,
        /// The **selection**: what an action would hit.
        pub(super) selected: SelectionState,
        /// Where a Shift-click counts from, as a position rather than an id,
        /// because a range is a span of the list and not of the mailbox.
        pub(super) anchor: Cell<Option<u32>>,
        /// The row the pointer is over, so the hover treatment can be taken
        /// off it when the pointer leaves.
        pub(super) hovered: RefCell<Option<MessageRowView>>,
        pub(super) view: gtk::ListView,
        pub(super) density: Rc<Cell<Density>>,
        pub(super) activated: RefCell<Vec<ActivateHandler>>,
        pub(super) cursor_moved: RefCell<Vec<CursorHandler>>,
        /// The message last reported to [`cursor_moved`]. Kept so the report
        /// is once per *message* rather than once per signal: the cursor
        /// landing and the row's page arriving are two different signals for
        /// the same landing, and a flag repaint is neither.
        ///
        /// [`cursor_moved`]: Self::cursor_moved
        pub(super) reported: Cell<Option<MessageId>>,
        /// Whether the user has put the cursor anywhere yet.
        ///
        /// `SingleSelection` autoselects row 0 the moment the model has rows,
        /// which is not somebody looking at a message — and the reading pane
        /// must not fill on startup for a row nobody chose. Once the dwell
        /// timer of #71 exists, that would also mark the newest message read
        /// for the sole reason that the application was opened, which is the
        /// unread signal destroying itself. Every real move sets this; the
        /// autoselect does not.
        pub(super) landed: Cell<bool>,
        pub(super) commands: RefCell<Vec<CommandHandler>>,
        /// `[ui].show_hover_actions`, handed to every row as it binds.
        pub(super) show_actions: Rc<Cell<bool>>,
        /// The live keymap, handed to every row as it binds so the focused
        /// row's key hints read the bindings actually in force.
        pub(super) keymap: Rc<RefCell<Keymap>>,
        /// The mailbox in view, so opening another one drops a selection that
        /// was about the last.
        pub(super) mailbox: RefCell<String>,
        /// What `meta` is currently saying, as a number rather than a
        /// sentence. Kept so that whatever replaces the header — a result
        /// count, a thread — can put the folder's own back afterwards
        /// without having to re-read the sidebar.
        pub(super) unread: std::cell::Cell<u32>,
        /// How a drag turns into files, when one is dropped outside Postio.
        ///
        /// Empty in a build that never wired it, which is why
        /// [`crate::drag_out::LazyFiles`] refuses rather than handing over
        /// an empty drop.
        pub(super) export: RefCell<Option<crate::drag_out::Materialise>>,
    }

    impl Default for MessageListView {
        fn default() -> Self {
            let model = MessageList::new();
            let cursor = gtk::SingleSelection::new(Some(model.clone()));
            MessageListView {
                title: gtk::Label::new(None),
                meta: gtk::Label::new(None),
                sort: gtk::Label::new(Some("Newest ▾")),
                count: gtk::Label::new(None),
                bulk: gtk::Box::new(gtk::Orientation::Horizontal, 6),
                view: gtk::ListView::new(Some(cursor.clone()), None::<gtk::ListItemFactory>),
                model,
                cursor,
                selected: SelectionState::new(),
                anchor: Cell::new(None),
                hovered: RefCell::new(None),
                density: Rc::new(Cell::new(Density::default())),
                activated: RefCell::new(Vec::new()),
                cursor_moved: RefCell::new(Vec::new()),
                reported: Cell::new(None),
                landed: Cell::new(false),
                commands: RefCell::new(Vec::new()),
                show_actions: Rc::new(Cell::new(true)),
                keymap: Rc::new(RefCell::new(Keymap::resolve(&Default::default()))),
                mailbox: RefCell::new(String::new()),
                unread: std::cell::Cell::new(0),
                export: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageListView {
        const NAME: &'static str = "PostioMessageListView";
        type Type = super::MessageListView;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for MessageListView {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for MessageListView {
        /// Focusing the pane means focusing a row.
        ///
        /// Without this the keyboard would stop at the scroller, which
        /// looks like focus and acts like nothing: no selected row, no key
        /// hints, and `j`/`k` with nowhere to go.
        fn grab_focus(&self) -> bool {
            self.view.grab_focus()
        }
    }
    impl BinImpl for MessageListView {}
}

glib::wrapper! {
    /// The list pane: a header, and a windowed list of rows under it.
    pub struct MessageListView(ObjectSubclass<imp::MessageListView>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for MessageListView {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl MessageListView {
    /// An empty list pane, with no mailbox named and no source attached.
    pub fn new() -> Self {
        Self::default()
    }

    /// The windowed model under the rows.
    pub fn model(&self) -> MessageList {
        self.imp().model.clone()
    }

    /// Feed this pane from `source`.
    ///
    /// The returned `Feed` is what opens a mailbox and what the runtime's
    /// events are handed to; the pane itself stays ignorant of where rows
    /// come from, which is what keeps `rusqlite` out of this crate.
    pub fn feed(&self, source: std::rc::Rc<dyn crate::feed::MessageSource>) -> crate::feed::Feed {
        crate::feed::Feed::new(&self.imp().model, source)
    }

    /// The cursor: which row the keyboard is on.
    ///
    /// `GtkSingleSelection` is GTK's name for it. What an action will hit is
    /// [`selection`](Self::selection), which is a different question — see
    /// [`crate::selection`].
    pub fn cursor(&self) -> gtk::SingleSelection {
        self.imp().cursor.clone()
    }

    /// What an action will hit.
    pub fn selection(&self) -> SelectionState {
        self.imp().selected.clone()
    }

    /// Called with every command the mouse runs — the bulk bar, a hover
    /// action, the context menu.
    pub fn connect_command(&self, handler: impl Fn(Command) + 'static) {
        self.imp().commands.borrow_mut().push(Box::new(handler));
    }

    /// Whether rows offer their actions under the pointer.
    ///
    /// `[ui].show_hover_actions`. Applied to the rows on screen now and to
    /// every row that binds after.
    pub fn set_show_actions(&self, show: bool) {
        if self.imp().show_actions.replace(show) == show {
            return;
        }
        self.each_row(|row| row.set_show_actions(show));
    }

    /// The bindings the focused row's key hints read.
    ///
    /// Applied to the rows on screen now and to every row that binds after,
    /// so a rebind in `config.toml` reaches the hints with no restart —
    /// the same promise already kept for the resolver, the palette and the
    /// cheat sheet.
    pub fn set_keymap(&self, keymap: Keymap) {
        self.imp().keymap.replace(keymap.clone());
        self.each_row(|row| row.set_keymap(&keymap));
    }

    /// The keymap in force, for the rows that bind after this and for a test
    /// to check against with nothing materialised on screen yet.
    pub fn keymap(&self) -> Keymap {
        self.imp().keymap.borrow().clone()
    }

    /// Name the mailbox in view and say how much of it is unread.
    ///
    /// Opening a different mailbox drops the selection with it: "these twelve"
    /// and "everything" both mean something else the moment the list does,
    /// and an action carrying a selection across that boundary would land on
    /// mail the user cannot see. `postio-core`'s `AppState::open_mailbox`
    /// makes the same decision on its side.
    pub fn set_mailbox(&self, name: &str, unread: u32) {
        let imp = self.imp();
        if imp.mailbox.replace(name.to_owned()) != name {
            self.imp().anchor.set(None);
            self.imp().selected.clear();
        }
        imp.unread.set(unread);
        imp.title.set_text(name);
        imp.meta.set_text(&match unread {
            0 => String::new(),
            n => format!("{n} unread"),
        });
        imp.meta.set_visible(unread > 0);
    }

    /// Where the list is scrolled to, in pixels.
    ///
    /// Exposed for the thread drill-in, which has to put it back: re-focusing
    /// the list on the way out scrolls the cursor row into view, and "into
    /// view" is not the same pixel offset the user left. See
    /// [`crate::window::Window::close_thread`].
    pub fn scroll_offset(&self) -> f64 {
        self.scroller()
            .map(|scroller| scroller.vadjustment().value())
            .unwrap_or(0.0)
    }

    /// Scroll to `offset`, clamped to what there is to scroll.
    pub fn set_scroll_offset(&self, offset: f64) {
        if let Some(scroller) = self.scroller() {
            scroller.vadjustment().set_value(offset);
        }
    }

    /// The `GtkScrolledWindow` the rows live in.
    ///
    /// Walked up rather than taken as the immediate parent: a
    /// `GtkScrolledWindow` puts a `GtkViewport` between itself and a child
    /// that is not scrollable, and whether it does is not something this
    /// wants to depend on.
    fn scroller(&self) -> Option<gtk::ScrolledWindow> {
        let mut widget = self.imp().view.parent();
        while let Some(current) = widget {
            if let Ok(scroller) = current.clone().downcast::<gtk::ScrolledWindow>() {
                return Some(scroller);
            }
            widget = current.parent();
        }
        None
    }

    /// The unread count the header is currently showing.
    ///
    /// The other half of [`mailbox_name`](Self::mailbox_name): together they
    /// are exactly what [`set_mailbox`](Self::set_mailbox) needs, so a
    /// surface that takes the header over can hand it back unchanged.
    pub fn unread(&self) -> u32 {
        self.imp().unread.get()
    }

    /// What the list column is currently calling its mailbox.
    ///
    /// The thread drill-in draws it into `Esc back to Inbox`, so the way out
    /// names where it goes rather than just promising there is one.
    pub fn mailbox_name(&self) -> String {
        self.imp().mailbox.borrow().clone()
    }

    /// Switch every row to `density`, live.
    ///
    /// A re-measure of the rows already on screen, never a rebuilt widget
    /// tree: the rows on screen are the rows that stay on screen.
    pub fn set_density(&self, density: Density) {
        let imp = self.imp();
        if imp.density.replace(density) == density {
            return;
        }
        self.each_row(|row| row.set_density(density));
    }

    /// The density currently in force.
    pub fn density(&self) -> Density {
        self.imp().density.get()
    }

    /// Called when a row is activated.
    pub fn connect_activated(&self, handler: impl Fn(crate::list::Row) + 'static) {
        self.imp().activated.borrow_mut().push(Box::new(handler));
    }

    /// Called when the cursor lands on a row.
    ///
    /// Every landing, in order, however the cursor got there — `j`, `k`, `G`,
    /// `g g`, or a click. This is what the reading pane follows: the preview
    /// tracks the cursor, and nothing waits for Return (issue #70).
    ///
    /// A move that goes nowhere reports nothing, so `k` on the first row does
    /// not make the reader re-read a message it is already showing. Toggling
    /// the selection reports nothing either — that moves no cursor.
    pub fn connect_cursor_moved(&self, handler: impl Fn(crate::list::Row) + 'static) {
        self.imp().cursor_moved.borrow_mut().push(Box::new(handler));
    }

    /// Tell the subscribers which message the cursor is on, if that changed.
    ///
    /// Deduplicated on the message id rather than on the signal, because
    /// three different things reach here: the cursor moving, the cursor's
    /// page arriving, and any other edit to the model — a flag toggled, a
    /// row repainted, a new message inserted above. Only the first two are
    /// a landing, and the id is what tells them apart.
    fn report_cursor(&self) {
        let imp = self.imp();
        let position = imp.cursor.selected();
        if !imp.landed.get() {
            // The autoselect, not a person. Remember where it put the cursor
            // so the first real move is still a change, but say nothing.
            imp.reported.set(imp.model.peek(position));
            return;
        }
        if position == gtk::INVALID_LIST_POSITION {
            imp.reported.set(None);
            return;
        }
        // A placeholder: the page under the cursor has not been delivered.
        // Nothing to show yet; `items_changed` brings us back when it lands.
        let Some(row) = imp
            .cursor
            .item(position)
            .and_downcast::<MessageRow>()
            .and_then(|item| item.row())
        else {
            return;
        };
        if imp.reported.replace(Some(row.id)) == Some(row.id) {
            return;
        }
        for handler in imp.cursor_moved.borrow().iter() {
            handler(row.clone());
        }
    }

    /// What a drag from this pane offers a receiver.
    ///
    /// Two offers in one drag, and the receiver picks. Postio's own sidebar
    /// takes the string, which stays a *reference* to the selection and never
    /// lists it; anything outside the application takes files, which are
    /// produced only if the drop lands there.
    ///
    /// Both describe the same mail: the drag source has already made the
    /// grabbed row the selection by the time it asks for this, so the ids
    /// below are exactly what the string half resolves to. Handing the file
    /// half a different set would move one thing into a folder and copy
    /// another to the desktop.
    ///
    /// Files are offered only for a selection that can be **named**.
    /// "Everything in this mailbox" is a predicate, and a predicate has no
    /// file form — resolving one would mean writing an `.eml` per message in
    /// a folder that may hold a hundred thousand, which is the one thing
    /// `spec.md` §18 says never to do. So a select-all drag still moves mail
    /// between folders inside Postio and offers nothing to the desktop: a
    /// drop that never highlights, rather than one that fails after the fact.
    ///
    /// Public because it is what the drag actually does, and a test that
    /// drove anything else would be testing a copy of it.
    pub fn drag_offer(&self) -> gdk::ContentProvider {
        let mut providers = vec![drag_payload()];
        if let (Some(export), Selection::These(messages)) = (
            self.imp().export.borrow().clone(),
            self.imp().selected.selection(),
        ) && !messages.is_empty()
        {
            providers.push(crate::drag_out::LazyFiles::for_messages(messages, export).upcast());
        }
        gdk::ContentProvider::new_union(&providers)
    }

    /// How a drag of these messages becomes files, for a drop outside Postio.
    ///
    /// The view layer cannot produce them: an `.eml` file is the raw message
    /// out of the blob store, and this crate may not depend on `rusqlite`. So
    /// the application registers the half that can, and the drag carries a
    /// promise rather than a payload — nothing is written unless a drop
    /// somewhere else actually asks. See [`crate::drag_out`].
    ///
    /// A build that never calls this still drags perfectly well *inside*
    /// Postio; it simply offers no files to anyone outside it.
    pub fn connect_export(&self, materialise: crate::drag_out::Materialise) {
        self.imp().export.replace(Some(materialise));
    }

    /// Move the keyboard one row down — `j`.
    pub fn next_row(&self) {
        self.step_cursor(1);
    }

    /// Move the keyboard one row up — `k`.
    pub fn prev_row(&self) {
        self.step_cursor(-1);
    }

    /// Move the keyboard to the first row — `g g`.
    pub fn first_row(&self) {
        self.move_cursor_to(0);
    }

    /// Move the keyboard to the last row — `G`.
    pub fn last_row(&self) {
        match self.imp().model.n_items() {
            0 => {}
            total => self.move_cursor_to(total - 1),
        }
    }

    /// Move the cursor without touching the selection.
    ///
    /// `j` and `k` move where the keyboard is and leave what an action would
    /// hit alone — the selection follows only when the user asks for it, with
    /// `Shift+J`. `postio-core`'s `AppState::focus_on` says the same thing on
    /// its side.
    fn step_cursor(&self, step: i32) {
        let imp = self.imp();
        let total = imp.model.n_items();
        if total == 0 {
            return;
        }
        let at = match imp.cursor.selected() {
            gtk::INVALID_LIST_POSITION => return self.move_cursor_to(0),
            at => at as i64 + step as i64,
        };
        if at < 0 || at >= total as i64 {
            return;
        }
        self.move_cursor_to(at as u32);
    }

    /// Toggle the row the keyboard is on — `x`, and Ctrl-click.
    pub fn toggle_cursor_row(&self) {
        if let Some(id) = self.cursor_id() {
            self.imp().anchor.set(Some(self.imp().cursor.selected()));
            self.imp().selected.toggle(id);
        }
    }

    /// Extend the selection onto the next row down — `J`.
    pub fn extend_down(&self) {
        self.extend_by(1);
    }

    /// Extend the selection onto the next row up — `K`.
    pub fn extend_up(&self) {
        self.extend_by(-1);
    }

    /// Select every message the list is showing — `Ctrl+A`.
    ///
    /// A predicate, not a hundred thousand ids: see [`crate::selection`].
    pub fn select_all(&self) {
        self.imp().anchor.set(None);
        self.imp().selected.select_all();
    }

    /// Drop the selection — `Esc`, when there is one.
    pub fn clear_selection(&self) {
        self.imp().anchor.set(None);
        self.imp().selected.clear();
    }

    /// The message the cursor is on, when the list has one.
    /// The whole row the cursor is on, not just its id.
    ///
    /// What the thread drill-in needs: `t` acts on the row's *thread*, and
    /// on its subject and thread count, none of which an id carries.
    pub fn cursor_row(&self) -> Option<crate::list::Row> {
        let position = self.cursor().selected();
        self.model()
            .item(position)
            .and_then(|item| item.downcast::<crate::list::MessageRow>().ok())
            .and_then(|item| item.row())
    }

    pub fn cursor_id(&self) -> Option<MessageId> {
        let imp = self.imp();
        imp.model.peek(imp.cursor.selected())
    }

    /// Extend the selection one row in `step`'s direction, taking the cursor
    /// with it.
    ///
    /// Both halves matter: `Shift+J` is "and this one too", so the row it
    /// lands on joins the selection *and* becomes where the keyboard is —
    /// which is what makes repeating it walk a range rather than flip the
    /// same pair of rows back and forth.
    fn extend_by(&self, step: i32) {
        let imp = self.imp();
        let from = imp.cursor.selected();
        if from == gtk::INVALID_LIST_POSITION {
            return;
        }
        // The row the cursor is on joins first: `Shift+J` from an empty
        // selection means both rows, not just the one below.
        if let Some(id) = imp.model.peek(from) {
            imp.selected.extend_to(id);
        }

        let to = from as i64 + step as i64;
        let total = imp.model.n_items() as i64;
        if to < 0 || to >= total {
            return;
        }
        let to = to as u32;
        imp.landed.set(true);
        imp.cursor.set_selected(to);
        imp.view.scroll_to(to, gtk::ListScrollFlags::FOCUS, None);
        if let Some(id) = imp.model.peek(to) {
            imp.selected.extend_to(id);
        }
    }

    /// Run `id` against whatever is selected — what the bulk bar means.
    fn run(&self, id: CommandId) {
        self.act(Command::default_for(id));
    }

    /// Run `id` against one row — what a hover action or the context menu on
    /// a row means, whether or not that row is in the selection.
    fn run_on(&self, id: CommandId, message: MessageId) {
        self.act(Command::default_for(id).with_target(MessageTarget::Messages(vec![message])));
    }

    /// Hand a command to whoever is listening.
    fn act(&self, command: Command) {
        for handler in self.imp().commands.borrow().iter() {
            handler(command.clone());
        }
    }

    /// Put the header back in step with the selection.
    fn refresh_header(&self) {
        let imp = self.imp();
        let total = match imp.model.n_items() {
            0 => None,
            total => Some(total),
        };
        match selection::summary(&imp.selected.selection(), total) {
            Some(text) => {
                imp.count.set_text(&text);
                imp.count.set_visible(true);
                imp.bulk.set_visible(true);
                // The count replaces the unread line rather than joining it:
                // while a selection is up, what an action will hit is the
                // only number that matters.
                imp.meta.set_visible(false);
                imp.sort.set_visible(false);
            }
            None => {
                imp.count.set_visible(false);
                imp.bulk.set_visible(false);
                imp.meta.set_visible(!imp.meta.text().is_empty());
                imp.sort.set_visible(true);
            }
        }
    }

    /// Put the rows on screen back in step with the selection.
    ///
    /// Only the realised ones: a row that is not on screen has nothing to
    /// repaint, and is told what it is the moment it binds.
    fn refresh_rows(&self) {
        let selected = self.imp().selected.clone();
        self.each_row(|row| {
            let is_selected = row.row().is_some_and(|row| selected.contains(row.id));
            row.set_selected(is_selected);
        });
    }

    /// Every row widget currently realised, for a live restyle.
    pub fn each_row(&self, handler: impl Fn(&MessageRowView)) {
        let mut child = self.imp().view.first_child();
        while let Some(item) = child {
            if let Some(row) = item.first_child().and_downcast::<MessageRowView>() {
                handler(&row);
            }
            child = item.next_sibling();
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-listview");

        imp.title.add_css_class("postio-list-title");
        imp.meta.add_css_class("postio-list-meta");
        imp.sort.add_css_class("postio-list-meta");

        imp.count.add_css_class("postio-list-meta");
        imp.count.add_css_class("postio-list-count");
        imp.count.set_visible(false);
        // A live region, so a screen reader hears "12 selected" as the
        // selection grows instead of only when something reads the header.
        imp.count.set_accessible_role(gtk::AccessibleRole::Status);

        imp.bulk.set_visible(false);
        imp.bulk.set_valign(gtk::Align::Center);
        for (id, title, key) in BULK_ACTIONS {
            let button = gtk::Button::builder()
                .tooltip_text(format!("{title} the selection"))
                .build();
            button.add_css_class("flat");
            button.add_css_class("postio-ghost");
            button.set_child(Some(&crate::header::labelled(title, key)));
            button.update_property(&[gtk::accessible::Property::Label(&format!(
                "{title} the selection"
            ))]);
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = pane)]
                self,
                move |_| pane.run(id)
            ));
            imp.bulk.append(&button);
        }

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("postio-list-header");
        header.set_valign(gtk::Align::Baseline);
        header.append(&imp.title);
        header.append(&imp.meta);
        header.append(&imp.count);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        header.append(&spacer);
        header.append(&imp.sort);
        header.append(&imp.bulk);
        // The header names the pane; the rows below it are the list itself,
        // so a screen reader hears "Inbox, 12 unread" once and then rows.
        header.set_accessible_role(gtk::AccessibleRole::Presentation);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let view = MessageRowView::new();
            // Not focusable *here*: the accessible row is the list item
            // around it, and the row must not take the keyboard out of it.
            // A focusable widget that also calls itself presentational is a
            // contradiction GTK does not survive — it stops painting.
            view.set_focusable(false);
            item.set_child(Some(&view));
            item.set_activatable(true);
            // The cursor lives on the list item and its state flag does not
            // reach the child, so it is handed down explicitly — and kept
            // in step, because a click on another row moves it off this one.
            // `is_selected` is GTK's word for "this is the current item";
            // what an action would hit is the other flag, set on bind.
            item.connect_selected_notify(glib::clone!(
                #[weak]
                view,
                move |item| view.set_cursor(item.is_selected())
            ));
        });
        let bound = imp.density.clone();
        let chosen = imp.selected.clone();
        // Shared with the pane the same way the density is: the factory
        // outlives any borrow of it, and a bind should cost a `Cell` read
        // rather than an upgrade through a weak reference.
        let offers = imp.show_actions.clone();
        let keymap = imp.keymap.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(view) = item.child().and_downcast::<MessageRowView>() else {
                return;
            };
            view.set_density(bound.get());
            view.set_show_actions(offers.get());
            view.set_keymap(&keymap.borrow());
            view.set_first(item.position() == 0);
            view.set_index(item.position());
            view.set_cursor(item.is_selected());
            view.set_row(
                item.item()
                    .and_downcast::<MessageRow>()
                    .and_then(|item| item.row()),
            );
            let selected = view.row().is_some_and(|row| chosen.contains(row.id));
            view.set_selected(selected);
            announce(item, &view, selected);
        });
        factory.connect_unbind(move |_, item| {
            if let Some(view) = item
                .downcast_ref::<gtk::ListItem>()
                .and_then(|item| item.child())
                .and_downcast::<MessageRowView>()
            {
                view.set_row(None);
                if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                    item.set_accessible_label("");
                }
            }
        });

        imp.view.set_factory(Some(&factory));
        imp.view.add_css_class("postio-rows");
        imp.view.set_show_separators(false);
        imp.view.set_single_click_activate(false);
        imp.view.set_vexpand(true);
        imp.view.connect_activate(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |view, position| {
                let Some(row) = view
                    .model()
                    .and_then(|model| model.item(position))
                    .and_downcast::<MessageRow>()
                    .and_then(|item| item.row())
                else {
                    return;
                };
                for handler in pane.imp().activated.borrow().iter() {
                    handler(row.clone());
                }
            }
        ));

        // The cursor announcing itself. `notify::selected` fires only when
        // the position actually changes, which is exactly the contract
        // `connect_cursor_moved` promises: `k` on the first row moves
        // nothing and so reports nothing, and the reader is not asked to
        // re-read a message it is already showing.
        //
        // This also covers the first landing. `SingleSelection` autoselects
        // row 0 as soon as the model has rows, so a window that has just
        // finished its first page tells the reader to show that message --
        // which is why the pane is no longer blank on startup.
        imp.cursor.connect_selected_notify(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| pane.report_cursor()
        ));

        // The other half. `SingleSelection` autoselects row 0 the moment the
        // model has *rows*, and `set_source` gives it placeholders before
        // `deliver` gives it mail -- so on a first page the cursor has
        // already landed by the time there is anything to show, and
        // `notify::selected` has been and gone. Without this the pane stays
        // blank until the user presses a key, which is #70's startup case.
        //
        // It is also the fast-scroll case: the cursor can sit on a page that
        // has not arrived yet, and the reader should fill in when it does.
        imp.model.connect_items_changed(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_, _, _, _| pane.report_cursor()
        ));

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        // Not a tab stop: the list inside it already scrolls with the
        // keyboard, so stopping here would be a stop that does nothing and
        // announces nothing.
        scroller.set_focusable(false);
        scroller.set_child(Some(&imp.view));
        scroller.set_vexpand(true);
        // Scrolls under a drag too, for the same reason the sidebar does: a
        // drag that started near the bottom of a long list has to be able to
        // reach the rest of it without being put down first.
        crate::autoscroll::attach(&scroller);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&scroller);
        self.set_child(Some(&column));

        imp.selected.connect_changed(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| {
                pane.refresh_rows();
                pane.refresh_header();
            }
        ));
        self.install_pointer();
    }

    /// The mouse's half of selecting, which has to reach everything the
    /// keyboard reaches (`/ux-architect`: mouse is equal, not lesser).
    ///
    /// | Gesture | Means |
    /// |---|---|
    /// | click | this row, and only this row |
    /// | click on the check | add or remove this row, cursor unmoved |
    /// | Ctrl-click | add or remove this row |
    /// | Shift-click | everything from the anchor to here |
    ///
    /// Handled in the capture phase because the answer is usually "not what
    /// GTK would have done": a modified click has to reach this before
    /// `GtkListView` turns it into a plain cursor move. A plain click is the
    /// exception — it is let through, so double-click still activates.
    fn install_pointer(&self) {
        let imp = self.imp();

        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |gesture, _, x, y| {
                if pane.press(gesture.current_event_state(), x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        ));
        imp.view.add_controller(click);

        // Right-click, from the registry rather than from a list written out
        // here: a menu that named its own verbs would be a third place to
        // keep them in step with the keys and the palette.
        let menu = gtk::GestureClick::new();
        menu.set_button(gdk::BUTTON_SECONDARY);
        menu.set_propagation_phase(gtk::PropagationPhase::Capture);
        menu.connect_pressed(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |gesture, _, x, y| {
                if pane.open_context_menu(x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        ));
        imp.view.add_controller(menu);

        // Dragging a row onto a folder moves it there — and what travels is
        // the *selection*, not the row under the cursor, whenever that row is
        // part of one. Dragging twelve selected messages and having one of
        // them move is the kind of thing people only notice after it has
        // happened to their mail.
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::MOVE);
        drag.connect_prepare(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            #[upgrade_or]
            None,
            move |source, x, y| {
                let (view, position) = pane.row_at(x, y)?;
                // A drag that starts on a hover action is not a drag: the
                // pointer is over a button, and pulling it sideways should
                // not take the message with it.
                let local = pane
                    .imp()
                    .view
                    .compute_point(&view, &graphene::Point::new(x as f32, y as f32));
                if local.is_some_and(|local| {
                    view.action_at(local.x() as f64, local.y() as f64).is_some()
                }) {
                    return None;
                }

                let id = view.row().map(|row| row.id)?;
                // Grabbing a row that is not in the selection makes it the
                // selection, which is what every desktop list does and what
                // makes the drag image honest: it is about to say how many
                // messages are moving, and the answer has to be the number
                // that actually moves.
                if !pane.imp().selected.contains(id) {
                    pane.imp().anchor.set(Some(position));
                    pane.imp().selected.select_only(id);
                    pane.move_cursor_to(position);
                }

                if let Some(icon) = pane.drag_icon() {
                    source.set_icon(Some(&icon), 12, 12);
                }

                // Two offers in one drag, and the receiver picks. Postio's own
                // sidebar takes the string, which stays a *reference* to the
                // selection and never lists it; anything outside the
                // application takes files, which are produced only if the drop
                // lands there. A union rather than a choice made here, because
                // at drag start there is no way to know where it will end.
                Some(pane.drag_offer())
            }
        ));
        imp.view.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_, x, y| pane.hover(pane.row_at(x, y).map(|(row, _)| row))
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_| pane.hover(None)
        ));
        imp.view.add_controller(motion);
    }

    /// Acts on a press, and says whether the pane took it.
    fn press(&self, state: gdk::ModifierType, x: f64, y: f64) -> bool {
        let Some((view, position)) = self.row_at(x, y) else {
            return false;
        };
        let Some(id) = view.row().map(|row| row.id) else {
            return false;
        };
        let imp = self.imp();

        let local = self
            .imp()
            .view
            .compute_point(&view, &graphene::Point::new(x as f32, y as f32));

        // A hover action is a button the row drew, so it takes the press
        // before any selection gesture does — clicking "archive" means
        // archive, whatever modifier happens to be held.
        if state.is_empty()
            && let Some(local) = local
            && let Some(action) = view.action_at(local.x() as f64, local.y() as f64)
        {
            self.run_on(command_for(action), id);
            return true;
        }

        let in_check =
            local.is_some_and(|local| view.is_in_check(local.x() as f64, local.y() as f64));

        match press_kind(state, in_check) {
            Press::Extend => {
                let from = imp.anchor.get().unwrap_or(position);
                let rows: Vec<Option<MessageId>> = (0..imp.model.n_items())
                    .map(|at| imp.model.peek(at))
                    .collect();
                imp.selected
                    .extend_over(selection::range(&rows, from as usize, position as usize));
                imp.anchor.set(Some(from));
                self.move_cursor_to(position);
                return true;
            }
            Press::Toggle => {
                imp.selected.toggle(id);
                imp.anchor.set(Some(position));
                self.move_cursor_to(position);
                return true;
            }
            // Inside the check: toggle without moving the keyboard, so a row
            // can be added to the selection without the reading pane
            // following the pointer around.
            Press::Check => {
                imp.selected.toggle(id);
                imp.anchor.set(Some(position));
                return true;
            }
            Press::Plain => {}
        }

        // A plain click moves the keyboard and gives the selection back. It
        // does *not* select the row it lands on, and that is the whole
        // difference between a cursor and a selection: reading your mail one
        // message at a time would otherwise put a bulk action bar over the
        // list on every click, and pressing `x` on the row you had just
        // clicked would take it *out* of a selection you never made.
        //
        // What an action hits when nothing is selected is the row the cursor
        // is on. `postio-core` keeps `focus` beside `selected` for exactly
        // that, and `AppState::focus_on` already says the selection follows
        // only when the user asks for it.
        //
        // `GtkListView` is left to move the cursor and to notice a second
        // click as an activation.
        imp.anchor.set(Some(position));
        imp.selected.clear();
        false
    }

    /// Move the keyboard to `position`, and the focus with it.
    fn move_cursor_to(&self, position: u32) {
        let imp = self.imp();
        imp.landed.set(true);
        imp.cursor.set_selected(position);
        imp.view
            .scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
        // Explicitly, as well as through `notify::selected`, because clicking
        // the row the cursor is already on changes no position and so emits
        // nothing — and that click is still someone asking to read it.
        // `report_cursor` deduplicates, so the two paths cannot double-fire.
        self.report_cursor();
    }

    /// The row under `(x, y)` in the list's coordinates, and where it sits.
    fn row_at(&self, x: f64, y: f64) -> Option<(MessageRowView, u32)> {
        let mut widget = self.imp().view.pick(x, y, gtk::PickFlags::DEFAULT)?;
        loop {
            if let Ok(row) = widget.clone().downcast::<MessageRowView>() {
                let index = row.index();
                return Some((row, index));
            }
            widget = widget.parent()?;
        }
    }

    /// The picture that follows the pointer during a drag: how many messages
    /// are moving, in words.
    ///
    /// A count rather than a ghost of the row, because the row under the
    /// cursor is not what is moving when a selection is — and a drag that
    /// looked like one message while carrying forty would be lying at exactly
    /// the moment it matters. `None` when there is nothing to say, which the
    /// caller reads as "use the default".
    fn drag_icon(&self) -> Option<gdk::Paintable> {
        let imp = self.imp();
        let total = match imp.model.n_items() {
            0 => None,
            total => Some(total),
        };
        let text = match selection::summary(&imp.selected.selection(), total)? {
            // "1 selected" is a count; "1 message" is what is being carried.
            count if count.starts_with("1 ") => "1 message".to_owned(),
            count => count.replace(" selected", " messages"),
        };

        let layout = self.create_pango_layout(Some(&text));
        let (width, height) = layout.pixel_size();
        let (pad_x, pad_y) = (10.0, 6.0);
        let (w, h) = (width as f32 + pad_x * 2.0, height as f32 + pad_y * 2.0);

        let snapshot = gtk::Snapshot::new();
        let rect = graphene::Rect::new(0.0, 0.0, w, h);
        snapshot.append_color(&self.drag_icon_ground(), &rect);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(pad_x, pad_y));
        snapshot.append_layout(&layout, &self.drag_icon_ink());
        snapshot.restore();
        snapshot.to_paintable(Some(&graphene::Size::new(w, h)))
    }

    /// The drag image's ground and ink, off the cascade like everything else.
    fn drag_icon_ground(&self) -> gdk::RGBA {
        self.style_probe(&["postio-row-edge", "selected"])
    }

    fn drag_icon_ink(&self) -> gdk::RGBA {
        self.style_probe(&["postio-row-ground", "check-mark"])
    }

    /// Read one role's colour off a throwaway node under this widget's
    /// classes — the same trick the row uses, for the same reason: a scheme
    /// change has to move this with everything else.
    fn style_probe(&self, classes: &[&str]) -> gdk::RGBA {
        let probe = gtk::Label::new(None);
        probe.set_css_classes(classes);
        probe.set_parent(self);
        let colour = probe.color();
        probe.unparent();
        colour
    }

    /// Open the row's context menu at `(x, y)`, and say whether there was a
    /// row there to open one for.
    ///
    /// The menu is generated from the command registry, filtered to what
    /// applies to a message in the list, with each item carrying the key that
    /// does the same thing — the same table the cheat sheet and the palette
    /// are built from, so a verb cannot appear in one and not another.
    fn open_context_menu(&self, x: f64, y: f64) -> bool {
        let Some((view, position)) = self.row_at(x, y) else {
            return false;
        };
        let Some(id) = view.row().map(|row| row.id) else {
            return false;
        };

        // Right-clicking a row outside the selection moves the keyboard to it
        // first, so what the menu is about is never in doubt. A row already
        // in the selection is left alone: right-clicking one of twelve
        // selected messages must not collapse the twelve.
        if !self.imp().selected.contains(id) {
            self.move_cursor_to(position);
        }

        let menu = gio::Menu::new();
        for spec in postio_core::registry::all() {
            if !spec.contexts.contains(postio_core::Context::List) || !is_message_action(spec.id) {
                continue;
            }
            let item = gio::MenuItem::new(Some(spec.title), None);
            item.set_action_and_target_value(
                Some("listrow.command"),
                Some(&spec.id.as_str().to_variant()),
            );
            menu.append_item(&item);
        }

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&self.imp().view);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        let actions = gio::SimpleActionGroup::new();
        let command = gio::SimpleAction::new("command", Some(glib::VariantTy::STRING));
        command.connect_activate(glib::clone!(
            #[weak(rename_to = pane)]
            self,
            move |_, parameter| {
                let Some(name) = parameter.and_then(|value| value.str().map(str::to_owned)) else {
                    return;
                };
                let Some(spec) = postio_core::registry::all().find(|spec| spec.id.as_str() == name)
                else {
                    return;
                };
                // The menu is about the row it was opened on, not about the
                // selection — unless that row is in the selection, in which
                // case the whole selection is what the user is pointing at.
                if pane.imp().selected.contains(id) {
                    pane.run(spec.id);
                } else {
                    pane.run_on(spec.id, id);
                }
            }
        ));
        actions.add_action(&command);
        popover.insert_action_group("listrow", Some(&actions));

        popover.connect_closed(|popover| popover.unparent());
        popover.popup();
        true
    }

    /// Put the hover treatment on `row` and take it off whatever had it.
    fn hover(&self, row: Option<MessageRowView>) {
        let previous = self.imp().hovered.replace(row.clone());
        if previous == row {
            return;
        }
        if let Some(previous) = previous {
            previous.set_hovered(false);
        }
        if let Some(row) = row {
            row.set_hovered(true);
        }
    }
}

/// What a drag of Postio's own messages carries, so a drop can tell it from
/// any other text that lands on a folder.
///
/// A string rather than a custom GType, because the same drag will grow other
/// formats — dragging *out* of the application means `message/rfc822`
/// materialised from the blob store, which is `postio-qhz.3`'s — and a
/// content provider can offer several spellings of one drag only if each is
/// something the toolkit already knows how to carry.
const DRAG_PREFIX: &str = "postio-messages:";

/// What the payload says when the drag is about the whole selection.
///
/// A selection can be the predicate `Everything { except }` over a hundred
/// thousand rows, and naming them to carry them three inches across the
/// window would be the one thing the predicate exists to prevent. So a drag
/// of the selection says *that*, and the drop turns it into
/// `MessageTarget::Selection` — the same target the `m` key uses, resolved
/// once by whoever can resolve it cheaply.
const DRAG_SELECTION: &str = "selection";

/// What was dragged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dragged {
    /// These messages, named.
    Messages(Vec<MessageId>),
    /// Whatever is selected, however large that is.
    Selection,
}

/// What a drop is carrying, or `None` if it did not come from Postio.
///
/// Anything else dropped on a folder — a selection of text, a file, a link —
/// reads as nothing and is refused, rather than being parsed for numbers that
/// happen to be in it.
pub fn dragged_messages(payload: &str) -> Option<Dragged> {
    let body = payload.strip_prefix(DRAG_PREFIX)?;
    if body == DRAG_SELECTION {
        return Some(Dragged::Selection);
    }
    let messages: Vec<MessageId> = body
        .split(',')
        .filter_map(|id| id.trim().parse::<i64>().ok())
        .map(MessageId::new)
        .collect();
    (!messages.is_empty()).then_some(Dragged::Messages(messages))
}

/// The payload for a drag of the selection.
fn drag_payload() -> gdk::ContentProvider {
    gdk::ContentProvider::for_value(&format!("{DRAG_PREFIX}{DRAG_SELECTION}").to_value())
}

/// Which command a hover action runs.
fn command_for(action: crate::row::RowAction) -> CommandId {
    match action {
        crate::row::RowAction::Archive => CommandId::Archive,
        crate::row::RowAction::Flag => CommandId::Flag,
        crate::row::RowAction::Delete => CommandId::Delete,
    }
}

/// Whether a command is one that acts on the message under the pointer.
///
/// The context menu is about a row, so it carries the verbs that take a
/// message and not the ones that move around the list or open a surface —
/// "next message" in a menu about *this* message is noise.
fn is_message_action(id: CommandId) -> bool {
    matches!(
        Command::default_for(id),
        Command::Archive { .. }
            | Command::Delete { .. }
            | Command::Move { .. }
            | Command::Flag { .. }
            | Command::MarkUnread { .. }
            | Command::AddLabel { .. }
            | Command::Reply { .. }
            | Command::ReplyAll { .. }
            | Command::Forward { .. }
            | Command::OpenMessage { .. }
            | Command::ArchiveThread { .. }
    )
}

/// What a press on a row means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    /// Everything from the anchor to here.
    Extend,
    /// Add or remove this row, and bring the keyboard with it.
    Toggle,
    /// Add or remove this row, leaving the keyboard where it is.
    Check,
    /// Move the keyboard here and give the selection back.
    Plain,
}

/// Which of the four a press is, from the modifiers and where it landed.
///
/// Shift beats Ctrl, because a range is the more specific request: someone
/// holding both has asked for one thing and can only mean the narrower.
/// Either modifier beats the check, because a modified click is deliberate
/// and the square it happened to land on is not.
fn press_kind(state: gdk::ModifierType, in_check: bool) -> Press {
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        Press::Extend
    } else if state.contains(gdk::ModifierType::CONTROL_MASK) {
        Press::Toggle
    } else if in_check {
        Press::Check
    } else {
        Press::Plain
    }
}

/// Tell a screen reader what the row is, including whether it is selected.
///
/// Said in the label rather than as an accessible *state*, for the reason the
/// module docs give: the accessible row is the `GtkListItemWidget`, and
/// pushing a property onto it from inside `bind` segfaults. `set_accessible_label`
/// is the API GTK provides for exactly this moment, and it takes a sentence.
///
/// So "Selected" leads the sentence, in front of "Unread" — the same shape
/// the row's own label already uses for state before content. The count is
/// announced separately by the header, which is a live region.
fn announce(item: &gtk::ListItem, view: &MessageRowView, selected: bool) {
    let spoken = view.spoken();
    item.set_accessible_label(&match (selected, spoken.is_empty()) {
        (_, true) => String::new(),
        (true, false) => format!("Selected, {spoken}"),
        (false, false) => spoken,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: gdk::ModifierType = gdk::ModifierType::empty();
    const SHIFT: gdk::ModifierType = gdk::ModifierType::SHIFT_MASK;
    const CTRL: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK;

    #[test]
    fn a_plain_click_on_a_row_is_a_cursor_move() {
        assert_eq!(press_kind(NONE, false), Press::Plain);
    }

    #[test]
    fn a_plain_click_on_the_check_is_the_mouse_way_to_select() {
        // The path that does not require knowing a modifier exists.
        assert_eq!(press_kind(NONE, true), Press::Check);
    }

    #[test]
    fn ctrl_toggles_wherever_it_lands() {
        assert_eq!(press_kind(CTRL, false), Press::Toggle);
        assert_eq!(press_kind(CTRL, true), Press::Toggle);
    }

    #[test]
    fn shift_extends_and_outranks_the_rest() {
        assert_eq!(press_kind(SHIFT, false), Press::Extend);
        assert_eq!(press_kind(SHIFT | CTRL, false), Press::Extend);
        assert_eq!(press_kind(SHIFT, true), Press::Extend);
    }

    #[test]
    fn a_drag_from_postio_names_its_messages() {
        assert_eq!(
            dragged_messages("postio-messages:3,7,11"),
            Some(Dragged::Messages(vec![
                MessageId::new(3),
                MessageId::new(7),
                MessageId::new(11)
            ]))
        );
    }

    #[test]
    fn a_drag_of_the_selection_refers_to_it_rather_than_listing_it() {
        // The whole point: forty thousand selected messages cross the window
        // as one word. Naming them to carry them is the thing the predicate
        // exists to prevent.
        assert_eq!(
            dragged_messages("postio-messages:selection"),
            Some(Dragged::Selection)
        );
    }

    #[test]
    fn anything_else_dropped_on_a_folder_is_refused() {
        // Text dragged from a browser, a file, a link. None of it is mail,
        // and none of it should be parsed for the numbers it happens to
        // contain — a drop that moved message 2026 because the user dragged
        // a date onto a folder would be indefensible.
        assert_eq!(dragged_messages("2026,1,2"), None);
        assert_eq!(dragged_messages("https://example.com/1"), None);
        assert_eq!(dragged_messages(""), None);
        assert_eq!(dragged_messages("postio-messages:"), None);
    }

    #[test]
    fn a_hover_action_is_the_key_that_does_the_same_thing() {
        use crate::row::RowAction;

        assert_eq!(command_for(RowAction::Archive), CommandId::Archive);
        assert_eq!(command_for(RowAction::Flag), CommandId::Flag);
        assert_eq!(command_for(RowAction::Delete), CommandId::Delete);
    }

    #[test]
    fn the_context_menu_carries_verbs_about_a_message_and_not_about_the_list() {
        assert!(is_message_action(CommandId::Archive));
        assert!(is_message_action(CommandId::Reply));
        assert!(!is_message_action(CommandId::NextMessage));
        assert!(!is_message_action(CommandId::CommandPalette));
        assert!(!is_message_action(CommandId::SelectAll));
    }

    #[test]
    fn every_context_menu_entry_is_a_registry_command() {
        // The menu is generated, so this is really a check that the filter
        // above has not drifted from the table it filters — a verb in the
        // menu that the registry does not know would have no key and no
        // palette entry.
        let listed: Vec<CommandId> = postio_core::registry::all()
            .filter(|spec| {
                spec.contexts.contains(postio_core::Context::List) && is_message_action(spec.id)
            })
            .map(|spec| spec.id)
            .collect();

        assert!(listed.contains(&CommandId::Archive));
        assert!(listed.contains(&CommandId::Delete));
        assert!(!listed.is_empty());
        for id in listed {
            assert!(
                postio_core::registry::all().any(|spec| spec.id == id),
                "{id:?} is in the menu and not in the registry"
            );
        }
    }

    #[test]
    fn the_lock_keys_are_not_modifiers_anybody_meant() {
        // Caps Lock and Num Lock ride along in the state of every press.
        // Reading either as a gesture would make selection depend on a light
        // on the keyboard.
        assert_eq!(
            press_kind(gdk::ModifierType::LOCK_MASK, false),
            Press::Plain
        );
        assert_eq!(
            press_kind(gdk::ModifierType::LOCK_MASK | SHIFT, false),
            Press::Extend
        );
    }
}
