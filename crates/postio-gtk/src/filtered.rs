//! The Filtered view (spec 007 US9, screen 21): a full view, not a dialog,
//! of what filing archived. Each row is one message with its reason; tabs
//! `1`-`7` narrow it to one reason; the focused row offers its restore;
//! and nothing here is deleted.
//!
//! It reads through the client's typed Filtered calls: the tabs' counts
//! once when it opens, and a page of fifty rows at a time as the person
//! scrolls -- never the whole of it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::FilteredRow;
use postio_core::{CommandId, Keymap};
use postio_focus::RowPicker;
use postio_model::MessageId;
use postio_ui::filtered;
use postio_widgets::widgets::keyhint::{self, KeyLine};
use postio_widgets::widgets::space::{S1, S3};
use postio_widgets::widgets::{Kind, Size};

use crate::bulk::Bulk;
use crate::list::RowObject;
use crate::list::RowWidget;

/// What a person asked of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilteredAction {
    /// Go back to the inbox.
    Back,
    /// Open this message over the view.
    Open {
        /// The message.
        message: MessageId,
        /// Its subject, for the dialog's title.
        subject: String,
    },
    /// Restore this message and never filter its sender.
    Restore(MessageId),
    /// Ask whether to sweep the inbox (FR-118).
    Sweep,
    /// A verb on the selection bar: run this command on what is selected.
    Verb(CommandId),
}

type Handler = Rc<dyn Fn(FilteredAction)>;

/// What a row of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A day's heading.
    Heading,
    /// The row at this index of `rows`.
    Row(usize),
    /// The child at this index of the open thread.
    Child(usize),
}

/// The thread open under one of the rows (spec 013 FR-009): its children are
/// drawn after the row, and the keyboard walks them like rows.
#[derive(Debug, Clone)]
struct Open {
    /// The expanded row, an index into `rows`.
    anchor: usize,
    /// The messages, oldest first; the unsent reply has none.
    members: Vec<MessageId>,
    children: Vec<postio_ui::thread_list::ChildRowView>,
}

/// The view. See the module.
pub struct FilteredView {
    client: Client,
    root: gtk::Box,
    tabs: gtk::Box,
    list: gtk::ListBox,
    footer: KeyLine,
    /// What the keys act on from the focused row or child.
    scope_footer: Rc<crate::scope_footer::ScopeFooter>,
    /// The bar over the footer while anything is selected.
    /// What stands in place of the rows when the tab has none.
    empty: gtk::Label,
    bar: Bulk,
    /// The keyboard and the selection: the list's rules over these rows
    /// (`postio_focus::RowPicker`), never a second set.
    picker: RefCell<RowPicker>,
    /// The sweep button's words and key.
    sweep_words: gtk::Box,
    keymap: RefCell<Keymap>,
    /// Which tab is showing, `0` for All.
    tab: Cell<usize>,
    /// Each tab's count, as last read.
    counts: Cell<[u32; 7]>,
    /// Whether the counts have been read once.
    counted: Cell<bool>,
    /// The rows listed, in order.
    rows: RefCell<Vec<FilteredRow>>,
    /// What each row of the list is: a day's heading, or the row at that
    /// index of `rows`.
    shown: RefCell<Vec<Slot>>,
    /// The thread open under one row, if one is.
    open: RefCell<Option<Open>>,
    /// Whether the last page read was full: there may be more.
    more: Cell<bool>,
    /// Moves with every re-read, so a page for an earlier one is dropped.
    generation: Cell<u64>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<FilteredView>>,
}

impl FilteredView {
    /// The view, reading through `client`, its keys from `keymap`.
    pub fn new(client: Client, keymap: &Keymap) -> Rc<Self> {
        let back = gtk::Button::new();
        postio_widgets::widgets::button::style(&back, Kind::Ghost, Size::Regular);
        back.add_css_class("focus-filtered-back");
        let back_row = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        back_row.append(&gtk::Image::from_icon_name("go-previous-symbolic"));
        back_row.append(&gtk::Label::new(Some("Inbox")));
        back.set_child(Some(&back_row));
        let title = gtk::Label::new(Some(filtered::TITLE));
        title.add_css_class("focus-filtered-title");
        let subtitle = gtk::Label::new(Some(filtered::SUBTITLE));
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("focus-filtered-subtitle");
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.append(&title);
        titles.append(&subtitle);
        // FR-118: filtering what is already in the inbox is a deliberate
        // command, and this is where filtering lives. It asks first.
        let sweep = gtk::Button::new();
        postio_widgets::widgets::button::style(&sweep, Kind::Ghost, Size::Regular);
        sweep.add_css_class("focus-filtered-sweep");
        let sweep_words = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sweep.set_child(Some(&sweep_words));
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-filtered-header");
        header.set_start_widget(Some(&back));
        header.set_center_widget(Some(&titles));
        header.set_end_widget(Some(&sweep));

        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        tabs.add_css_class("focus-filtered-tabs");
        let note = gtk::Label::new(Some(filtered::NOTE));
        note.add_css_class("dim-label");
        note.add_css_class("focus-filtered-note");
        note.set_hexpand(true);
        note.set_xalign(1.0);
        note.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let tab_row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        tab_row.add_css_class("focus-filtered-tab-row");
        tab_row.append(&tabs);
        tab_row.append(&note);

        let list = gtk::ListBox::new();
        list.add_css_class("focus-filtered-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let footer = KeyLine::new("focus-filtered-footer");
        let empty = gtk::Label::new(Some(filtered::EMPTY));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        empty.set_focusable(true);
        empty.set_visible(false);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("focus-filtered");
        root.append(&header);
        root.append(&tab_row);
        root.append(&scrolled);
        root.append(&empty);
        // While anything is selected: screen 01's bar itself, with
        // Filtered's verbs.
        let bar = Bulk::with_verbs(filtered::BAR_VERBS, keymap);
        bar.widget().add_css_class("focus-filtered-bar");
        root.append(bar.widget());
        let scope_footer = Rc::new(crate::scope_footer::ScopeFooter::new());
        root.append(scope_footer.widget());
        root.append(footer.widget());

        let view = Rc::new(FilteredView {
            client,
            root,
            tabs,
            list,
            footer,
            scope_footer,
            empty,
            bar,
            picker: RefCell::default(),
            sweep_words,
            keymap: RefCell::new(keymap.clone()),
            tab: Cell::new(0),
            counts: Cell::new([0; 7]),
            counted: Cell::new(false),
            rows: RefCell::default(),
            shown: RefCell::default(),
            open: RefCell::default(),
            more: Cell::new(false),
            generation: Cell::new(0),
            handler: RefCell::default(),
            me: RefCell::default(),
        });
        view.me.replace(Rc::downgrade(&view));
        let weak = Rc::downgrade(&view);
        view.bar.connect_command({
            let weak = weak.clone();
            move |command| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Verb(command));
                }
            }
        });
        back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Back);
                }
            }
        });
        sweep.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Sweep);
                }
            }
        });
        view.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if let Some(found) = view.row_at(row.index()) {
                    view.emit(FilteredAction::Open {
                        message: found.message.id,
                        subject: found.message.subject.clone().unwrap_or_default(),
                    });
                } else if let Some((anchor, message)) = view.child_message_at(row.index()) {
                    view.emit(FilteredAction::Open {
                        message,
                        subject: anchor.message.subject.clone().unwrap_or_default(),
                    });
                }
            }
        });
        view.list.connect_row_selected({
            let weak = weak.clone();
            move |list, selected| {
                // Only the focused row offers its restore (screen 21).
                let mut child = list.first_child();
                while let Some(row) = child {
                    child = row.next_sibling();
                    if let Some(button) = restore_button(&row) {
                        let none_selected = weak.upgrade().is_none_or(|view| !view.has_selection());
                        button.set_visible(
                            none_selected && selected.is_some_and(|selected| *selected == row),
                        );
                    }
                }
                // A child shows the keys that act on it while it is focused.
                let mut child = list.first_child();
                while let Some(row) = child {
                    child = row.next_sibling();
                    if let Some(widget) = row.first_child().and_downcast::<RowWidget>() {
                        widget.set_cursor_here(selected.is_some_and(|selected| *selected == row));
                    }
                }
                if let Some(view) = weak.upgrade() {
                    let index = selected.and_then(|row| view.owner_of_row(row.index()));
                    view.picker.borrow_mut().focus(index.map(|at| at as u32));
                    view.show_scope();
                }
            }
        });
        // ⌘/Ctrl-click toggles a row and Shift-click takes a range, as the
        // list's rows do; a plain click is the list's own.
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed({
            let weak = weak.clone();
            move |gesture, _, _, y| {
                let state = gesture.current_event_state();
                let range = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                let toggle = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
                if !range && !toggle {
                    return;
                }
                let Some(view) = weak.upgrade() else {
                    return;
                };
                let Some(row) = view.list.row_at_y(y as i32) else {
                    return;
                };
                let Some(index) = view.index_of_row(row.index()) else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let change = view.picker.borrow_mut().pick(index as u32, range);
                view.apply(change);
            }
        });
        view.list.add_controller(click);
        scrolled.connect_edge_reached({
            let weak = weak.clone();
            move |_, edge| {
                if edge == gtk::PositionType::Bottom
                    && let Some(view) = weak.upgrade()
                    && view.more.get()
                {
                    view.read_page(view.rows.borrow().len() as u32);
                }
            }
        });
        view.set_keymap(keymap);
        view
    }

    /// The commands the view has a control for: Back, the sweep, the
    /// restore, and the tabs.
    pub fn controls() -> Vec<CommandId> {
        [
            CommandId::Back,
            CommandId::SweepInbox,
            CommandId::RestoreFiltered,
            CommandId::ToggleRead,
        ]
        .into_iter()
        .chain(filtered::TAB_COMMANDS)
        .collect()
    }

    /// The view, for the window's pages.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Run `handler` with what the person asks.
    pub fn connect_action(&self, handler: impl Fn(FilteredAction) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.bar.set_keymap(keymap);
        while let Some(child) = self.sweep_words.first_child() {
            self.sweep_words.remove(&child);
        }
        self.sweep_words.append(&keyhint::labelled(
            filtered::SWEEP_BUTTON,
            postio_ui::hints::key(keymap, CommandId::SweepInbox).as_deref(),
        ));
        self.footer.set(&filtered::footer(keymap, true));
        self.show_tabs();
        self.show_rows();
    }

    /// Open on All: read the tabs' counts and the first page.
    pub fn open(&self) {
        self.tab.set(0);
        self.refresh();
    }

    /// Read the counts and the tab on screen again: after a restore, an
    /// undo, or mail filed away.
    pub fn refresh(&self) {
        self.read_tabs();
        self.read_page(0);
    }

    /// Show the `index`th tab, `0` for All: `1`-`7`.
    pub fn set_tab(&self, index: usize) {
        if filtered::tab_reason(index).is_none() {
            return;
        }
        self.open.replace(None);
        self.tab.set(index);
        // Another place, as another folder is: what was selected is not
        // shown any more.
        let change = self.picker.borrow_mut().clear();
        self.apply(change);
        // And it opens on its first row, as every place does.
        self.picker.borrow_mut().set_rows(Vec::new());
        self.show_tabs();
        self.read_page(0);
    }

    /// The tab on screen, `0` for All.
    pub fn tab(&self) -> usize {
        self.tab.get()
    }

    /// The focused row's message, if any: what `R` restores.
    pub fn focused(&self) -> Option<MessageId> {
        let row = self.list.selected_row()?;
        let index = self.owner_of_row(row.index())?;
        self.rows.borrow().get(index).map(|found| found.message.id)
    }

    /// The listed rows' subjects, top to bottom.
    pub fn subjects(&self) -> Vec<String> {
        self.rows
            .borrow()
            .iter()
            .map(|row| row.message.subject.clone().unwrap_or_default())
            .collect()
    }

    /// Every text the view shows, in order: what a test reads.
    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.root.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && !label.text().is_empty()
            {
                said.push(label.text().to_string());
            }
            let mut child = widget.last_child();
            while let Some(previous) = child {
                child = previous.prev_sibling();
                stack.push(previous);
            }
        }
        said
    }

    /// Move the focus `by` rows, over the day headings: `j` and `k`.
    pub fn step(&self, by: i32) {
        let rows = self.shown.borrow().clone();
        let mut at = self.list.selected_row().map_or(-1, |row| row.index());
        loop {
            at += by;
            let Some(slot) = usize::try_from(at).ok().and_then(|at| rows.get(at)) else {
                return;
            };
            if *slot != Slot::Heading {
                if let Some(row) = self.list.row_at_index(at) {
                    self.list.select_row(Some(&row));
                    row.grab_focus();
                }
                return;
            }
        }
    }

    /// Open the focused row: `Enter`.
    pub fn open_focused(&self) {
        if let Some(row) = self.list.selected_row() {
            row.activate();
        }
    }

    /// Give the list the keyboard, on its first row.
    pub fn focus_list(&self) {
        if self.list.selected_row().is_none()
            && let Some(first) = self.list_row(0)
        {
            self.list.select_row(Some(&first));
        }
        match self.list.selected_row() {
            Some(row) => {
                row.grab_focus();
            }
            None => {
                self.list.grab_focus();
            }
        }
    }

    /// The commands the view runs itself: the selection's, which the
    /// controller's list behind it must never take.
    pub fn runs(&self, id: CommandId) -> bool {
        matches!(
            id,
            CommandId::ExpandThread
                | CommandId::CollapseThread
                | CommandId::ToggleSelection
                | CommandId::ExtendSelectionDown
                | CommandId::ExtendSelectionUp
                | CommandId::SelectAll
        ) || (id == CommandId::Back && self.has_selection())
    }

    /// Run a selection command, drawing what it changed; `j`/`k` stay with
    /// [`Self::step`].
    pub fn run(&self, id: CommandId) {
        match id {
            CommandId::ExpandThread => return self.expand_focused(),
            CommandId::CollapseThread => return self.collapse(),
            _ => {}
        }
        let change = self.picker.borrow_mut().run(id);
        self.apply(change);
    }

    /// Whether anything is selected.
    pub fn has_selection(&self) -> bool {
        !self.picker.borrow().selected().is_empty()
    }

    /// The messages a verb is about: the selection, else the focused row.
    pub fn aimed(&self) -> Vec<MessageId> {
        self.picker.borrow().aimed()
    }

    /// The messages selected.
    pub fn selected(&self) -> Vec<MessageId> {
        self.picker.borrow().selected()
    }

    /// What a verb sent has acted on goes from the selection.
    pub fn clear_selection(&self) {
        let change = self.picker.borrow_mut().clear();
        self.apply(change);
    }

    /// Draw a change the picker made: the keyboard's row, the boxes, the bar.
    fn apply(&self, change: postio_focus::PickChange) {
        if let Some(at) = change.focus
            && let Some(row) = self.list_row(at as usize)
        {
            self.list.select_row(Some(&row));
            row.grab_focus();
        }
        if change.selection.is_some() {
            self.show_picks();
        }
    }

    /// The index in `rows` of the list row at `index`, over day headings.
    fn index_of_row(&self, index: i32) -> Option<usize> {
        let at = usize::try_from(index).ok()?;
        match self.shown.borrow().get(at).copied()? {
            Slot::Row(index) => Some(index),
            _ => None,
        }
    }

    /// The row of `rows` the list row at `index` belongs to: itself, or for
    /// a child the row it hangs from.
    fn owner_of_row(&self, index: i32) -> Option<usize> {
        let at = usize::try_from(index).ok()?;
        match self.shown.borrow().get(at).copied()? {
            Slot::Row(index) => Some(index),
            Slot::Child(_) => self.open.borrow().as_ref().map(|open| open.anchor),
            Slot::Heading => None,
        }
    }

    /// The row a child at list row `index` hangs from, and its message.
    fn child_message_at(&self, index: i32) -> Option<(FilteredRow, MessageId)> {
        let at = usize::try_from(index).ok()?;
        let Slot::Child(child) = self.shown.borrow().get(at).copied()? else {
            return None;
        };
        let open = self.open.borrow();
        let open = open.as_ref()?;
        let anchor = self.rows.borrow().get(open.anchor).cloned()?;
        Some((anchor, *open.members.get(child)?))
    }

    /// Check the selected rows' boxes and bring the bar up or down.
    fn show_picks(&self) {
        let selected = self.picker.borrow().selected();
        let mut at = 0;
        let mut child = self.list.first_child();
        while let Some(row) = child {
            child = row.next_sibling();
            let message = self
                .index_of_row(at)
                .and_then(|index| self.rows.borrow().get(index).map(|found| found.message.id));
            at += 1;
            let picked = message.is_some_and(|message| selected.contains(&message));
            if picked {
                row.add_css_class("focus-filtered-picked");
            } else {
                row.remove_css_class("focus-filtered-picked");
            }
            // One key, one scope: with a selection, R is the bar's.
            if let Some(button) = find_class(&row, "focus-filtered-restore") {
                button.set_visible(
                    selected.is_empty()
                        && self
                            .list
                            .selected_row()
                            .is_some_and(|on| on.index() == at - 1),
                );
            }
            if let Some(check) = find_class(&row, "focus-filtered-check") {
                check.set_opacity(if picked { 1.0 } else { 0.0 });
            }
        }
        self.show_bar(&selected);
    }

    /// The footer's bar, for a test to read.
    pub fn scope_footer(&self) -> &Rc<crate::scope_footer::ScopeFooter> {
        &self.scope_footer
    }

    /// Say what the keys act on from the focused row or child, in the
    /// words the list's footer uses (`postio_ui::scope::footer`).
    fn show_scope(&self) {
        use postio_ui::scope::{FooterAbout, ScopeFocus, footer};
        let Some(selected) = self.list.selected_row() else {
            self.scope_footer
                .set(&postio_ui::scope::FooterView::default());
            return;
        };
        let at = usize::try_from(selected.index()).ok();
        let slot = at.and_then(|at| self.shown.borrow().get(at).copied());
        let rows = self.rows.borrow();
        let open = self.open.borrow();
        let view = match slot {
            Some(Slot::Row(index)) => rows.get(index).map(|row| {
                let message = &row.message;
                let thread = message.thread.unwrap_or(postio_model::ThreadId::new(0));
                let count = message.thread_count.max(1) as usize;
                footer(
                    ScopeFocus::Row {
                        thread,
                        latest: message.id,
                    },
                    &FooterAbout {
                        subject: message.subject.as_deref().unwrap_or_default(),
                        count,
                        index: count - 1,
                        sender: "",
                    },
                    &self.keymap.borrow(),
                )
            }),
            Some(Slot::Child(child)) => open.as_ref().and_then(|open| {
                let row = rows.get(open.anchor)?;
                let message = *open.members.get(child)?;
                let sender = &open.children.get(child)?.sender;
                footer(
                    ScopeFocus::Child {
                        thread: row.message.thread.unwrap_or(postio_model::ThreadId::new(0)),
                        message,
                    },
                    &FooterAbout {
                        subject: row.message.subject.as_deref().unwrap_or_default(),
                        count: open.members.len(),
                        index: child,
                        sender,
                    },
                    &self.keymap.borrow(),
                )
                .into()
            }),
            _ => None,
        };
        self.scope_footer.set(&view.unwrap_or_default());
    }

    /// `Right`: open the focused row's thread under it, when it has more
    /// than one message.
    fn expand_focused(&self) {
        let Some(row) = self.list.selected_row() else {
            return;
        };
        if let Some(index) = self.index_of_row(row.index()) {
            self.expand(index);
        }
    }

    fn toggle(&self, index: usize) {
        if self
            .open
            .borrow()
            .as_ref()
            .is_some_and(|open| open.anchor == index)
        {
            self.collapse();
        } else {
            self.expand(index);
        }
    }

    /// Read the thread of the row at `index` and draw its messages under it,
    /// folding any other.
    fn expand(&self, index: usize) {
        let Some(row) = self.rows.borrow().get(index).cloned() else {
            return;
        };
        let Some(thread) = row.message.thread.filter(|_| row.message.thread_count > 1) else {
            return;
        };
        if self
            .open
            .borrow()
            .as_ref()
            .is_some_and(|open| open.anchor == index)
        {
            return;
        }
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        let generation = self.generation.get();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.thread(thread).await;
            let (Some(view), Ok(Some(thread))) = (weak.upgrade(), read) else {
                return;
            };
            if view.generation.get() != generation {
                return;
            }
            let now = postio_ui::clock::now();
            let keymap = view.keymap.borrow().clone();
            let mut children: Vec<_> = (0..thread.members.len())
                .filter_map(|at| {
                    postio_ui::thread_list::ChildRowView::member(&thread, at, &[], &keymap, now)
                })
                .collect();
            children.extend(postio_ui::thread_list::ChildRowView::draft(&thread, now));
            if thread.members.len() < 2 {
                return;
            }
            view.open.replace(Some(Open {
                anchor: index,
                members: thread.members.iter().map(|member| member.id).collect(),
                children,
            }));
            view.show_rows();
            if let Some(row) = view.list_row(index) {
                view.list.select_row(Some(&row));
                row.grab_focus();
            }
        });
    }

    /// `Left`: fold the open thread, the keyboard back on its row.
    fn collapse(&self) {
        let Some(open) = self.open.take() else {
            return;
        };
        self.show_rows();
        if let Some(row) = self.list_row(open.anchor) {
            self.list.select_row(Some(&row));
            row.grab_focus();
        }
    }

    /// Say so when the tab on screen has nothing, once it has been read.
    fn show_empty(&self) {
        let nothing = self.counted.get() && self.rows.borrow().is_empty() && !self.more.get();
        self.empty.set_visible(nothing);
        self.footer
            .set(&filtered::footer(&self.keymap.borrow(), !nothing));
        if nothing {
            // The rows the keyboard was on are gone: it stays in the view.
            self.empty.grab_focus();
        }
    }

    fn show_bar(&self, selected: &[MessageId]) {
        if selected.is_empty() {
            self.bar.set_summary(None);
            return;
        }
        let summary = postio_ui::selection::summary(
            &postio_core::state::Selection::These(selected.to_vec()),
            None,
            &[],
        );
        self.bar.set_summary(Some(&summary.unwrap_or_default()));
        crate::a11y::teach_shortcuts(self.bar.widget());
    }

    fn emit(&self, action: FilteredAction) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }

    fn row_at(&self, index: i32) -> Option<FilteredRow> {
        let at = usize::try_from(index).ok()?;
        let Slot::Row(index) = self.shown.borrow().get(at).copied()? else {
            return None;
        };
        self.rows.borrow().get(index).cloned()
    }

    /// The list row that shows the `index`th of `rows`.
    fn list_row(&self, index: usize) -> Option<gtk::ListBoxRow> {
        let at = self
            .shown
            .borrow()
            .iter()
            .position(|shown| *shown == Slot::Row(index))?;
        self.list.row_at_index(i32::try_from(at).ok()?)
    }

    /// Read the tabs' counts.
    fn read_tabs(&self) {
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.filtered_tabs().await;
            let (Some(view), Ok(reasons)) = (weak.upgrade(), read) else {
                return;
            };
            view.counts.set(filtered::tab_counts(&reasons));
            view.counted.set(true);
            view.show_empty();
            view.show_tabs();
        });
    }

    /// Read the tab on screen from `offset`: the first page replaces what
    /// is listed, a later one adds to it.
    fn read_page(&self, offset: u32) {
        let generation = if offset == 0 {
            self.generation.get() + 1
        } else {
            self.generation.get()
        };
        self.generation.set(generation);
        let reason = filtered::tab_reason(self.tab.get())
            .flatten()
            .map(str::to_owned);
        let client = self.client.clone();
        let weak = self.me.borrow().clone();
        self.more.set(false);
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: as `read_tabs`'.
            let read = client.filtered(reason, offset, filtered::PAGE).await;
            let Some(view) = weak.upgrade() else {
                return;
            };
            if view.generation.get() != generation {
                return;
            }
            let page = match read {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "Focus could not read Filtered: {error}");
                    return;
                }
            };
            view.more.set(filtered::page_is_full(page.len()));
            {
                let mut rows = view.rows.borrow_mut();
                if offset == 0 {
                    rows.clear();
                }
                rows.extend(page);
            }
            // Keep the keyboard on its message, or on the row that took its
            // place, and what is selected.
            let ids = view
                .rows
                .borrow()
                .iter()
                .map(|row| row.message.id)
                .collect();
            view.picker.borrow_mut().set_rows(ids);
            view.show_rows();
            let index = view.picker.borrow().focused().unwrap_or(0) as usize;
            if let Some(row) = view.list_row(index) {
                view.list.select_row(Some(&row));
            }
        });
    }

    /// Draw the tabs with their counts, the one on screen raised.
    fn show_tabs(&self) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        let counts = self.counts.get();
        for (index, (_, name)) in filtered::TABS.iter().enumerate() {
            let words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            let label = gtk::Label::new(Some(name));
            words.append(&label);
            let count = gtk::Label::new(Some(&counts[index].to_string()));
            count.add_css_class("dim-label");
            words.append(&count);
            let tab = gtk::Button::new();
            tab.set_child(Some(&words));
            postio_widgets::widgets::button::style(&tab, Kind::Ghost, Size::Regular);
            tab.add_css_class("focus-filtered-tab");
            if index == self.tab.get() {
                tab.add_css_class("focus-filtered-tab-on");
            }
            if let Some(key) =
                postio_ui::hints::key(&self.keymap.borrow(), filtered::TAB_COMMANDS[index])
            {
                tab.set_tooltip_text(Some(&filtered::tab_tooltip(name, &key)));
            }
            let weak = self.me.borrow().clone();
            tab.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.set_tab(index);
                }
            });
            self.tabs.append(&tab);
        }
    }

    /// Draw the rows, each day under its heading.
    fn show_rows(&self) {
        crate::rebuild::keeping_focus(&self.list, || self.rebuild_rows());
    }

    fn rebuild_rows(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let now = postio_ui::clock::now();
        let today = now.date_naive();
        let restore_key = postio_ui::hints::key(&self.keymap.borrow(), CommandId::RestoreFiltered);
        let rows = self.rows.borrow();
        let days: Vec<_> = rows
            .iter()
            .map(|row| row.at.with_timezone(&chrono::Local).date_naive())
            .collect();
        let headings = filtered::day_headings(&days, today);
        let mut shown = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some(said) = &headings[index] {
                let heading = gtk::Label::new(Some(said));
                heading.add_css_class("focus-day-heading");
                heading.add_css_class("focus-filtered-day");
                heading.set_xalign(0.0);
                let item = gtk::ListBoxRow::new();
                item.set_child(Some(&heading));
                item.set_selectable(false);
                item.set_activatable(false);
                item.set_can_focus(false);
                self.list.append(&item);
                shown.push(Slot::Heading);
            }
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
            line.add_css_class("focus-filtered-row");
            // The list's checked box, in the gutter.
            let check = gtk::Image::from_icon_name("checkbox-checked-symbolic");
            check.add_css_class("focus-filtered-check");
            // Always laid out, so a checked row's columns line up with the
            // rest; only drawn while it is selected.
            check.set_opacity(0.0);
            line.append(&check);
            // A conversation of more than one message has a chevron, and
            // says how many (spec 013 FR-009).
            let thread = row.message.thread_count > 1;
            let chevron = gtk::Image::from_icon_name(
                if self
                    .open
                    .borrow()
                    .as_ref()
                    .is_some_and(|open| open.anchor == index)
                {
                    "pan-down-symbolic"
                } else {
                    "pan-end-symbolic"
                },
            );
            chevron.add_css_class("focus-filtered-chevron");
            chevron.set_opacity(if thread { 1.0 } else { 0.0 });
            if thread {
                let click = gtk::GestureClick::new();
                click.set_propagation_phase(gtk::PropagationPhase::Capture);
                let weak = self.me.borrow().clone();
                click.connect_pressed(move |gesture, _, _, _| {
                    if let Some(view) = weak.upgrade() {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        view.toggle(index);
                    }
                });
                chevron.add_controller(click);
            }
            line.append(&chevron);
            let sender = gtk::Label::new(Some(
                &row.message
                    .from
                    .as_ref()
                    .map(|from| from.display().to_owned())
                    .unwrap_or_default(),
            ));
            sender.add_css_class("focus-filtered-sender");
            sender.set_xalign(0.0);
            sender.set_width_chars(22);
            sender.set_max_width_chars(22);
            sender.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&sender);
            if thread
                && let Some(count) = postio_ui::focus_row::count_badge(row.message.thread_count)
            {
                let badge = gtk::Label::new(Some(&count));
                badge.add_css_class("dim-label");
                badge.add_css_class("focus-filtered-count");
                line.append(&badge);
            }
            let subject = gtk::Label::new(Some(row.message.subject.as_deref().unwrap_or_default()));
            subject.add_css_class("focus-filtered-subject");
            subject.set_xalign(0.0);
            subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&subject);
            let preview = gtk::Label::new(row.message.preview.as_deref());
            preview.add_css_class("dim-label");
            preview.set_xalign(0.0);
            preview.set_hexpand(true);
            preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&preview);
            let restore = gtk::Button::new();
            restore.add_css_class("focus-filtered-restore");
            restore.set_child(Some(&keyhint::labelled(
                filtered::RESTORE,
                restore_key.as_deref(),
            )));
            restore.set_visible(false);
            restore.set_valign(gtk::Align::Center);
            let message = row.message.id;
            let weak = self.me.borrow().clone();
            restore.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade() {
                    view.emit(FilteredAction::Restore(message));
                }
            });
            line.append(&restore);
            let pill = gtk::Label::new(Some(&filtered::pill(&row.reason, row.source.as_deref())));
            pill.add_css_class("focus-filtered-pill");
            pill.set_valign(gtk::Align::Center);
            line.append(&pill);
            let time = gtk::Label::new(Some(&postio_ui::row::timestamp(row.at, now)));
            time.add_css_class("focus-filtered-time");
            line.append(&time);

            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&line));
            self.list.append(&item);
            shown.push(Slot::Row(index));
            if let Some(open) = self
                .open
                .borrow()
                .as_ref()
                .filter(|open| open.anchor == index)
            {
                for (at, child) in open.children.iter().enumerate() {
                    let widget = RowWidget::default();
                    let last = at + 1 == open.children.len();
                    widget.bind(&RowObject::child(child.clone(), last));
                    let item = gtk::ListBoxRow::new();
                    item.set_child(Some(&widget));
                    item.add_css_class("focus-filtered-child");
                    self.list.append(&item);
                    shown.push(Slot::Child(at));
                }
            }
        }
        drop(rows);
        self.shown.replace(shown);
        self.show_picks();
        self.show_empty();
        // Rows are rebuilt here, including after `read_page`'s async
        // fetch lands, so a cap taught only once after `open()` would miss
        // every row a later page adds (T142, T143).
        crate::a11y::teach_shortcuts(&self.root);
        crate::motion::keep_to_budget(&self.root);
    }
}

/// The restore button in a row of the list, wherever the day heading put
/// it.
fn restore_button(row: &gtk::Widget) -> Option<gtk::Button> {
    let mut stack = vec![row.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class("focus-filtered-restore") {
            return widget.downcast().ok();
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    None
}

/// The first widget under `root` with CSS class `class`.
fn find_class(root: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) {
            return Some(widget);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next);
        }
    }
    None
}
