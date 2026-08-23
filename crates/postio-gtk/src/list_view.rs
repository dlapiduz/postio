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
//! spec.md §18 forbids: a 100,000-message folder would cost 100,000 widgets.
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
use gtk::{gdk, glib, graphene};
use postio_config::Density;
use postio_core::CommandId;
use postio_model::MessageId;

use crate::list::{MessageList, MessageRow};
use crate::row::MessageRowView;
use crate::selection::{self, SelectionState};

/// What to call when a row is activated — `Enter`, or a double click.
type ActivateHandler = Box<dyn Fn(crate::list::Row)>;

/// What to call when the bulk bar is used — the same command ids the keyboard
/// resolves to, so the two paths cannot drift.
type CommandHandler = Box<dyn Fn(CommandId)>;

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
        pub(super) commands: RefCell<Vec<CommandHandler>>,
        /// The mailbox in view, so opening another one drops a selection that
        /// was about the last.
        pub(super) mailbox: RefCell<String>,
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
                commands: RefCell::new(Vec::new()),
                mailbox: RefCell::new(String::new()),
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
    /// The returned [`Feed`] is what opens a mailbox and what the runtime's
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

    /// Called with every command the bulk bar runs.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().commands.borrow_mut().push(Box::new(handler));
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
        imp.title.set_text(name);
        imp.meta.set_text(&match unread {
            0 => String::new(),
            n => format!("{n} unread"),
        });
        imp.meta.set_visible(unread > 0);
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
        imp.cursor.set_selected(to);
        imp.view.scroll_to(to, gtk::ListScrollFlags::FOCUS, None);
        if let Some(id) = imp.model.peek(to) {
            imp.selected.extend_to(id);
        }
    }

    /// Run `id` through whoever is listening — the bulk bar's buttons and the
    /// keyboard end up in the same place.
    fn run(&self, id: CommandId) {
        for handler in self.imp().commands.borrow().iter() {
            handler(id);
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
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(view) = item.child().and_downcast::<MessageRowView>() else {
                return;
            };
            view.set_density(bound.get());
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

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        // Not a tab stop: the list inside it already scrolls with the
        // keyboard, so stopping here would be a stop that does nothing and
        // announces nothing.
        scroller.set_focusable(false);
        scroller.set_child(Some(&imp.view));
        scroller.set_vexpand(true);

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

        let in_check = self
            .imp()
            .view
            .compute_point(&view, &graphene::Point::new(x as f32, y as f32))
            .is_some_and(|local| view.is_in_check(local.x() as f64, local.y() as f64));

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
        imp.cursor.set_selected(position);
        imp.view
            .scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
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
