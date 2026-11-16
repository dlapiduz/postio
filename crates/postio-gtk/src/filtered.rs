//! The Filtered view (spec 007 US9, screen 21): a full view, not a dialog,
//! of what filing archived. Each row is one message with its reason; tabs
//! `1`-`7` narrow it to one reason; the focused row offers its restore;
//! and nothing here is deleted.
//!
//! The view only draws. What it shows -- the tabs, the rows read a page at
//! a time, the row with the keyboard (a display position, the thread's
//! children counted), the selection, the thread open under a row -- is the
//! controller's (`postio_focus`, ADR 0045, spec 013 FR-025), told to it as
//! inputs and drawn from its intents, so the rule is written once for both
//! platforms.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};
use postio_focus::{FilteredLine, FilteredSplice, FilteredView as Frame, Input};
use postio_model::MessageId;
use postio_ui::filtered;
use postio_widgets::widgets::keyhint::{self, KeyLine};
use postio_widgets::widgets::space::{S1, S3};
use postio_widgets::widgets::{Kind, Size};

use crate::bulk::Bulk;
use crate::list::RowObject;
use crate::list::RowWidget;

/// What the view asks of the window, which owns the rest: a sweep is
/// confirmed in a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilteredAction {
    /// Ask whether to sweep the inbox (FR-118).
    Sweep,
}

type Handler = Rc<dyn Fn(FilteredAction)>;

/// What a row of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A day's heading.
    Heading,
    /// The row at this index of the frame's rows.
    Row(usize),
    /// The child at this index of the open thread.
    Child(usize),
}

/// The view. See the module.
pub struct FilteredView {
    root: gtk::Box,
    tabs: gtk::Box,
    list: gtk::ListBox,
    footer: KeyLine,
    /// What the keys act on from the focused row or child.
    scope_footer: Rc<crate::scope_footer::ScopeFooter>,
    /// What stands in place of the rows when the tab has none.
    empty: gtk::Label,
    bar: Bulk,
    /// The sweep button's words and key.
    sweep_words: gtk::Box,
    /// What the controller last drew: the rows, the splice, the keyboard
    /// and the selection as intents have changed them since.
    frame: RefCell<Option<Frame>>,
    /// What each row of the list is.
    shown: RefCell<Vec<Slot>>,
    /// The list row each display position is drawn in.
    at: RefCell<Vec<i32>>,
    /// Set while the view moves the list's own selection, which is not a
    /// click.
    drawing: Cell<bool>,
    /// Tells the controller what a person did.
    send: Rc<dyn Fn(Input)>,
    /// Runs a command in the controller.
    command: Rc<dyn Fn(CommandId)>,
    handler: RefCell<Option<Handler>>,
    me: RefCell<std::rc::Weak<FilteredView>>,
}

impl FilteredView {
    /// The view, its keys from `keymap`. `send` tells the controller a
    /// click or a scroll; `command` runs a command in it.
    pub fn new(
        keymap: &Keymap,
        send: impl Fn(Input) + 'static,
        command: impl Fn(CommandId) + 'static,
    ) -> Rc<Self> {
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
            root,
            tabs,
            list,
            footer,
            scope_footer,
            empty,
            bar,
            sweep_words,
            frame: RefCell::default(),
            shown: RefCell::default(),
            at: RefCell::default(),
            drawing: Cell::new(false),
            send: Rc::new(send),
            command: Rc::new(command),
            handler: RefCell::default(),
            me: RefCell::default(),
        });
        view.me.replace(Rc::downgrade(&view));
        let weak = Rc::downgrade(&view);
        view.bar.connect_command({
            let weak = weak.clone();
            move |command| {
                if let Some(view) = weak.upgrade() {
                    (view.command)(command);
                }
            }
        });
        back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(view) = weak.upgrade() {
                    (view.command)(CommandId::GoToInbox);
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
                if let Some(position) = view.position_of_row(row.index()) {
                    (view.send)(Input::FilteredPoint(position));
                    (view.command)(CommandId::OpenMessage);
                }
            }
        });
        view.list.connect_row_selected({
            let weak = weak.clone();
            move |_, selected| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                if view.drawing.get() {
                    return;
                }
                if let Some(position) = selected.and_then(|row| view.position_of_row(row.index())) {
                    (view.send)(Input::FilteredPoint(position));
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
                let Some(position) = view.position_of_row(row.index()) else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                (view.send)(Input::FilteredPick { position, range });
            }
        });
        view.list.add_controller(click);
        scrolled.connect_edge_reached({
            let weak = weak.clone();
            move |_, edge| {
                if edge == gtk::PositionType::Bottom
                    && let Some(view) = weak.upgrade()
                {
                    (view.send)(Input::FilteredMore);
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

    /// Read every key from `keymap`: the bar's verbs. The tabs, the sweep
    /// and the restore take theirs from the next frame.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.bar.set_keymap(keymap);
        while let Some(child) = self.sweep_words.first_child() {
            self.sweep_words.remove(&child);
        }
        self.sweep_words.append(&keyhint::labelled(
            filtered::SWEEP_BUTTON,
            postio_ui::hints::key(keymap, CommandId::SweepInbox).as_deref(),
        ));
    }

    /// Forget what the last visit drew, so that coming back shows nothing
    /// until the controller's first frame lands rather than rows it has
    /// since moved on from.
    pub fn clear(&self) {
        self.frame.replace(None);
        self.show_tabs();
        self.show_rows();
        self.show_empty();
        self.show_bar();
    }

    /// Draw `frame`, whole: `Intent::Filtered`.
    pub fn show(&self, frame: &Frame) {
        self.frame.replace(Some(frame.clone()));
        self.footer.set(&frame.footer);
        while let Some(child) = self.sweep_words.first_child() {
            self.sweep_words.remove(&child);
        }
        self.sweep_words
            .append(&keyhint::labelled(&frame.sweep, frame.sweep_key.as_deref()));
        self.show_tabs();
        self.show_rows();
        self.show_empty();
        self.show_bar();
    }

    /// The keyboard is on display position `position` now:
    /// `Intent::FilteredFocus`.
    pub fn show_focus(&self, position: Option<u32>) {
        if let Some(frame) = self.frame.borrow_mut().as_mut() {
            frame.focused = position;
        }
        self.place_focus();
    }

    /// Open `splice` under its row: `Intent::FilteredSplice`.
    pub fn show_splice(&self, anchor: u32, children: &[postio_ui::thread_list::ChildRowView]) {
        if let Some(frame) = self.frame.borrow_mut().as_mut() {
            frame.expanded = Some(FilteredSplice {
                anchor,
                children: children.to_vec(),
            });
        }
        self.show_rows();
    }

    /// Fold the thread under `anchor`: `Intent::FilteredUnsplice`.
    pub fn show_unsplice(&self, anchor: u32) {
        if let Some(frame) = self.frame.borrow_mut().as_mut()
            && frame
                .expanded
                .as_ref()
                .is_some_and(|open| open.anchor == anchor)
        {
            frame.expanded = None;
        }
        self.show_rows();
    }

    /// The selection changed: `Intent::FilteredSelection`.
    pub fn show_selection(&self, selected: &[MessageId], bar: Option<&postio_focus::FilteredBar>) {
        if let Some(frame) = self.frame.borrow_mut().as_mut() {
            for row in &mut frame.rows {
                row.selected = selected.contains(&row.message);
            }
            frame.bar = bar.cloned();
        }
        self.show_picks();
        self.show_bar();
    }

    /// What the keys act on from the focused row or child:
    /// `Intent::Footer`.
    pub fn show_scope(&self, view: &postio_ui::scope::FooterView) {
        self.scope_footer.set(view);
    }

    /// The focused row's message, if any: what `R` restores. A child is
    /// its thread's row.
    pub fn focused(&self) -> Option<MessageId> {
        let frame = self.frame.borrow();
        let frame = frame.as_ref()?;
        let row = self.slot_of_position(frame.focused?)?;
        let row = match row {
            Slot::Row(row) => row,
            Slot::Child(_) => frame.expanded.as_ref()?.anchor as usize,
            Slot::Heading => return None,
        };
        frame.rows.get(row).map(|line| line.message)
    }

    /// The listed rows' subjects, top to bottom.
    pub fn subjects(&self) -> Vec<String> {
        self.frame.borrow().as_ref().map_or_else(Vec::new, |frame| {
            frame.rows.iter().map(|row| row.subject.clone()).collect()
        })
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

    /// Give the list the keyboard, on the controller's row.
    pub fn focus_list(&self) {
        let focused = self
            .frame
            .borrow()
            .as_ref()
            .and_then(|frame| frame.focused)
            .and_then(|position| self.list_row_at(position));
        match focused {
            Some(row) => {
                row.grab_focus();
            }
            None => {
                self.list.grab_focus();
            }
        }
    }

    /// Whether anything is selected.
    pub fn has_selection(&self) -> bool {
        !self.selected().is_empty()
    }

    /// The messages selected.
    pub fn selected(&self) -> Vec<MessageId> {
        self.frame.borrow().as_ref().map_or_else(Vec::new, |frame| {
            frame
                .rows
                .iter()
                .filter(|row| row.selected)
                .map(|row| row.message)
                .collect()
        })
    }

    /// The footer's bar, for a test to read.
    pub fn scope_footer(&self) -> &Rc<crate::scope_footer::ScopeFooter> {
        &self.scope_footer
    }

    /// The subject the message at the keyboard's row is titled by: its own,
    /// or the open thread's for a child.
    pub fn focused_subject(&self) -> Option<String> {
        let frame = self.frame.borrow();
        let frame = frame.as_ref()?;
        let row = match self.slot_of_position(frame.focused?)? {
            Slot::Row(row) => row,
            Slot::Child(_) => frame.expanded.as_ref()?.anchor as usize,
            Slot::Heading => return None,
        };
        frame.rows.get(row).map(|line| line.subject.clone())
    }

    /// The slot display position `position` draws.
    fn slot_of_position(&self, position: u32) -> Option<Slot> {
        let at = *self.at.borrow().get(position as usize)?;
        self.shown.borrow().get(usize::try_from(at).ok()?).copied()
    }

    /// The display position the list row at `index` draws.
    fn position_of_row(&self, index: i32) -> Option<u32> {
        self.at
            .borrow()
            .iter()
            .position(|at| *at == index)
            .map(|position| position as u32)
    }

    /// The list row that draws display position `position`.
    fn list_row_at(&self, position: u32) -> Option<gtk::ListBoxRow> {
        let at = *self.at.borrow().get(position as usize)?;
        self.list.row_at_index(at)
    }

    /// Move the list's own selection to the controller's row, and show the
    /// focused row's restore and its keys.
    fn place_focus(&self) {
        let focused = self.frame.borrow().as_ref().and_then(|frame| frame.focused);
        self.drawing.set(true);
        match focused.and_then(|position| self.list_row_at(position)) {
            Some(row) => {
                self.list.select_row(Some(&row));
                row.grab_focus();
            }
            None => self.list.unselect_all(),
        }
        self.drawing.set(false);
        self.show_focus_marks();
    }

    /// Only the focused row offers its restore (screen 21), and a child
    /// shows the keys that act on it while it is focused.
    fn show_focus_marks(&self) {
        let selected = self.list.selected_row();
        let none_selected = !self.has_selection();
        let mut child = self.list.first_child();
        while let Some(row) = child {
            child = row.next_sibling();
            let here = selected.as_ref().is_some_and(|selected| *selected == row);
            if let Some(button) = restore_button(&row) {
                button.set_visible(none_selected && here);
            }
            if let Some(widget) = row.first_child().and_downcast::<RowWidget>() {
                widget.set_cursor_here(here);
            }
        }
    }

    /// Check the selected rows' boxes and bring the restore up or down.
    fn show_picks(&self) {
        let selected = self.selected();
        let shown = self.shown.borrow().clone();
        let frame = self.frame.borrow();
        let mut at = 0;
        let mut child = self.list.first_child();
        while let Some(row) = child {
            child = row.next_sibling();
            let picked = match (shown.get(at), frame.as_ref()) {
                (Some(Slot::Row(index)), Some(frame)) => frame
                    .rows
                    .get(*index)
                    .is_some_and(|line| selected.contains(&line.message)),
                _ => false,
            };
            at += 1;
            if picked {
                row.add_css_class("focus-filtered-picked");
            } else {
                row.remove_css_class("focus-filtered-picked");
            }
            if let Some(check) = find_class(&row, "focus-filtered-check") {
                check.set_opacity(if picked { 1.0 } else { 0.0 });
            }
        }
        drop(frame);
        self.show_focus_marks();
    }

    /// Say so when the tab on screen has nothing, once it has been read.
    fn show_empty(&self) {
        let said = self
            .frame
            .borrow()
            .as_ref()
            .and_then(|frame| frame.empty.clone());
        if let Some(said) = &said {
            self.empty.set_text(said);
        }
        self.empty.set_visible(said.is_some());
        if said.is_some() {
            // The rows the keyboard was on are gone: it stays in the view.
            self.empty.grab_focus();
        }
    }

    fn show_bar(&self) {
        let summary = self
            .frame
            .borrow()
            .as_ref()
            .and_then(|frame| frame.bar.as_ref().map(|bar| bar.summary.clone()));
        self.bar.set_summary(summary.as_deref());
        if summary.is_some() {
            crate::a11y::teach_shortcuts(self.bar.widget());
        }
    }

    fn emit(&self, action: FilteredAction) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(action);
        }
    }

    /// Draw the tabs with their counts, the one on screen raised.
    fn show_tabs(&self) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        let frame = self.frame.borrow();
        let Some(frame) = frame.as_ref() else {
            return;
        };
        for (index, tab) in frame.tabs.iter().enumerate() {
            let words = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            words.append(&gtk::Label::new(Some(&tab.name)));
            let count = gtk::Label::new(Some(&tab.count.to_string()));
            count.add_css_class("dim-label");
            words.append(&count);
            let button = gtk::Button::new();
            button.set_child(Some(&words));
            postio_widgets::widgets::button::style(&button, Kind::Ghost, Size::Regular);
            button.add_css_class("focus-filtered-tab");
            if tab.on {
                button.add_css_class("focus-filtered-tab-on");
            }
            if let Some(key) = &tab.key {
                button.set_tooltip_text(Some(&filtered::tab_tooltip(&tab.name, key)));
            }
            let weak = self.me.borrow().clone();
            button.connect_clicked(move |_| {
                if let Some(view) = weak.upgrade()
                    && let Some(command) = filtered::TAB_COMMANDS.get(index)
                {
                    (view.command)(*command);
                }
            });
            self.tabs.append(&button);
        }
    }

    /// Draw the rows, each day under its heading.
    fn show_rows(&self) {
        crate::rebuild::keeping_focus(&self.list, || self.rebuild_rows());
        self.place_focus();
    }

    fn rebuild_rows(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let frame = self.frame.borrow();
        let Some(frame) = frame.as_ref() else {
            self.shown.replace(Vec::new());
            self.at.replace(Vec::new());
            return;
        };
        let mut shown = Vec::new();
        let mut at = Vec::new();
        for (index, row) in frame.rows.iter().enumerate() {
            if let Some(said) = &row.heading {
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
            let open = frame
                .expanded
                .as_ref()
                .is_some_and(|open| open.anchor as usize == index);
            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&self.row(frame, index, row, open)));
            self.list.append(&item);
            at.push(shown.len() as i32);
            shown.push(Slot::Row(index));
            if let Some(open) = frame
                .expanded
                .as_ref()
                .filter(|open| open.anchor as usize == index)
            {
                for (child, view) in open.children.iter().enumerate() {
                    let widget = RowWidget::default();
                    let last = child + 1 == open.children.len();
                    widget.bind(&RowObject::child(view.clone(), last));
                    let item = gtk::ListBoxRow::new();
                    item.set_child(Some(&widget));
                    item.add_css_class("focus-filtered-child");
                    self.list.append(&item);
                    at.push(shown.len() as i32);
                    shown.push(Slot::Child(child));
                }
            }
        }
        self.shown.replace(shown);
        self.at.replace(at);
        // Rows are rebuilt here, including after a later page lands, so a
        // cap taught only once after the view opened would miss every row
        // a later page adds (T142, T143).
        crate::a11y::teach_shortcuts(&self.root);
        crate::motion::keep_to_budget(&self.root);
    }

    /// One row, as a line.
    fn row(&self, frame: &Frame, index: usize, row: &FilteredLine, open: bool) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        line.add_css_class("focus-filtered-row");
        // The list's checked box, in the gutter.
        let check = gtk::Image::from_icon_name("checkbox-checked-symbolic");
        check.add_css_class("focus-filtered-check");
        // Always laid out, so a checked row's columns line up with the
        // rest; only drawn while it is selected.
        check.set_opacity(if row.selected { 1.0 } else { 0.0 });
        line.append(&check);
        // A conversation of more than one message has a chevron, and says
        // how many (spec 013 FR-009).
        let thread = row.messages > 1;
        let chevron = gtk::Image::from_icon_name(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        chevron.add_css_class("focus-filtered-chevron");
        chevron.set_opacity(if thread { 1.0 } else { 0.0 });
        if thread {
            let click = gtk::GestureClick::new();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = self.me.borrow().clone();
            click.connect_pressed(move |gesture, _, _, _| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let position = view.display_of_row(index);
                if let Some(position) = position {
                    (view.send)(Input::FilteredPoint(position));
                    let open = view
                        .frame
                        .borrow()
                        .as_ref()
                        .and_then(|frame| frame.expanded.as_ref().map(|open| open.anchor))
                        == Some(index as u32);
                    (view.command)(if open {
                        CommandId::CollapseThread
                    } else {
                        CommandId::ExpandThread
                    });
                }
            });
            chevron.add_controller(click);
        }
        line.append(&chevron);
        let sender = gtk::Label::new(Some(&row.sender));
        sender.add_css_class("focus-filtered-sender");
        sender.set_xalign(0.0);
        sender.set_width_chars(22);
        sender.set_max_width_chars(22);
        sender.set_ellipsize(gtk::pango::EllipsizeMode::End);
        line.append(&sender);
        if thread && let Some(count) = postio_ui::focus_row::count_badge(row.messages) {
            let badge = gtk::Label::new(Some(&count));
            badge.add_css_class("dim-label");
            badge.add_css_class("focus-filtered-count");
            line.append(&badge);
        }
        let subject = gtk::Label::new(Some(&row.subject));
        subject.add_css_class("focus-filtered-subject");
        subject.set_xalign(0.0);
        subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
        line.append(&subject);
        let preview = gtk::Label::new(row.preview.as_deref());
        preview.add_css_class("dim-label");
        preview.set_xalign(0.0);
        preview.set_hexpand(true);
        preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
        line.append(&preview);
        let restore = gtk::Button::new();
        restore.add_css_class("focus-filtered-restore");
        restore.set_child(Some(&keyhint::labelled(
            &frame.restore,
            frame.restore_key.as_deref(),
        )));
        restore.set_visible(false);
        restore.set_valign(gtk::Align::Center);
        let weak = self.me.borrow().clone();
        restore.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade()
                && let Some(position) = view.display_of_row(index)
            {
                (view.send)(Input::FilteredPoint(position));
                (view.command)(CommandId::RestoreFiltered);
            }
        });
        line.append(&restore);
        let pill = gtk::Label::new(Some(&row.pill));
        pill.add_css_class("focus-filtered-pill");
        pill.set_valign(gtk::Align::Center);
        line.append(&pill);
        let time = gtk::Label::new(Some(&row.time));
        time.add_css_class("focus-filtered-time");
        line.append(&time);
        line
    }

    /// The display position of row `index`, children counted.
    fn display_of_row(&self, index: usize) -> Option<u32> {
        let frame = self.frame.borrow();
        let frame = frame.as_ref()?;
        let children = frame
            .expanded
            .as_ref()
            .filter(|open| (open.anchor as usize) < index)
            .map_or(0, |open| open.children.len());
        Some((index + children) as u32)
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
