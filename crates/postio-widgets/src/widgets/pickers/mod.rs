//! A 380 px popover anchored to the focused row: title, short list, a field and a footnote.
//! `Picker` is the frame all four pickers share; its keys come from the keymap's `Context::Picker`.
//! It decides nothing: the picker built on it gives a choice meaning.

mod when;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_core::{CommandId, Frontend, Keymap};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};

use crate::widgets::keyhint;
use crate::widgets::space::{S1, S2};

pub use when::{When, WhenPicker};

pub const WIDTH: i32 = 380;

const CHOOSE: [CommandId; 4] = [
    CommandId::PickerChoose1,
    CommandId::PickerChoose2,
    CommandId::PickerChoose3,
    CommandId::PickerChoose4,
];

#[derive(Debug, Clone)]
pub enum Mark {
    Dot(gdk::RGBA),
}

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub section: Option<String>,
    pub mark: Option<Mark>,
    pub name: String,
    pub detail: String,
    /// The first four numbered rows answer to `1`-`4`, in order.
    pub numbered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Date,
    /// Holds the keyboard from the start; its placeholder says what it filters.
    Filter(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Choose(usize),
    Confirm {
        typed: String,
        selected: Option<usize>,
    },
    Toggle(usize),
}

type Handler = Rc<dyn Fn(Picked)>;

pub struct Picker {
    popover: gtk::Popover,
    target: gtk::Label,
    entry: gtk::Entry,
    field_hint: gtk::Label,
    list: gtk::ListBox,
    footnote: gtk::Label,
    field: Field,
    keymap: RefCell<Keymap>,
    resolver: RefCell<Resolver>,
    rows: RefCell<Vec<Row>>,
    shown: RefCell<Vec<Option<usize>>>,
    handler: RefCell<Option<Handler>>,
    outside: RefCell<Option<(glib::WeakRef<gtk::Widget>, gtk::GestureClick)>>,
    /// Where the keyboard returns when the picker closes.
    before: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
}

impl Picker {
    pub fn new(keymap: &Keymap, title: &str, field: Field, footnote: &str) -> Rc<Self> {
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("postio-picker-title");
        heading.set_xalign(0.0);
        let target = gtk::Label::new(None);
        target.add_css_class("postio-picker-target");
        target.set_hexpand(true);
        target.set_xalign(1.0);
        target.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        title_row.add_css_class("postio-picker-head");
        title_row.append(&heading);
        title_row.append(&target);

        let entry = gtk::Entry::new();
        entry.add_css_class("postio-picker-entry");
        let field_hint = gtk::Label::new(None);
        field_hint.add_css_class("postio-picker-field-hint");
        field_hint.set_xalign(0.0);
        let list = gtk::ListBox::new();
        list.add_css_class("postio-picker-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(320)
            .build();
        let footnote_label = gtk::Label::new(Some(footnote));
        footnote_label.add_css_class("postio-picker-footnote");
        footnote_label.set_wrap(true);
        footnote_label.set_xalign(0.0);
        footnote_label.set_max_width_chars(1);

        let content = gtk::Box::new(gtk::Orientation::Vertical, S1);
        content.add_css_class("postio-picker");
        content.set_width_request(WIDTH);
        content.append(&title_row);
        match field {
            Field::Filter(placeholder) => {
                entry.set_placeholder_text(Some(placeholder));
                entry.set_primary_icon_name(Some("system-search-symbolic"));
                content.append(&entry);
                content.append(&scrolled);
            }
            Field::Date => {
                entry.set_placeholder_text(Some(postio_ui::pickers::DATE_PLACEHOLDER));
                let date = gtk::Box::new(gtk::Orientation::Vertical, 0);
                date.add_css_class("postio-picker-date");
                date.append(&entry);
                date.append(&field_hint);
                content.append(&scrolled);
                content.append(&date);
            }
        }
        content.append(&footnote_label);

        let popover = gtk::Popover::builder()
            .child(&content)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .build();
        popover.add_css_class("postio-picker-popover");
        // No grab: a key-opened grabbing popup is dismissed at once where nothing gave the window a
        // pointer serial (every headless run). The root focus holds the keyboard and a press
        // outside closes it (see `open`).
        popover.set_autohide(false);

        let (resolver, _) = Resolver::from_commands_for(keymap, Frontend::Focus);
        let picker = Rc::new(Picker {
            popover,
            target,
            entry,
            field_hint,
            list,
            footnote: footnote_label,
            field,
            keymap: RefCell::new(keymap.clone()),
            resolver: RefCell::new(resolver),
            rows: RefCell::default(),
            shown: RefCell::default(),
            handler: RefCell::default(),
            outside: RefCell::default(),
            before: RefCell::default(),
        });
        picker.popover.connect_closed({
            let weak = Rc::downgrade(&picker);
            move |popover| {
                let Some(picker) = weak.upgrade() else { return };
                let before = picker.before.take().and_then(|before| before.upgrade());
                // Only when the keyboard is still in the picker (or nowhere):
                // a close that already put it somewhere has said where.
                let held = popover
                    .root()
                    .and_then(|root| root.focus())
                    .is_none_or(|focus| {
                        focus == *popover.upcast_ref::<gtk::Widget>() || focus.is_ancestor(popover)
                    });
                if held && let Some(before) = before.filter(|before| before.is_mapped()) {
                    before.grab_focus();
                }
            }
        });
        let weak = Rc::downgrade(&picker);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, state| match weak.upgrade() {
                Some(picker) if picker.press(key, state) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            }
        });
        content.add_controller(keys);
        picker.entry.connect_activate({
            let weak = weak.clone();
            move |entry| {
                if let Some(picker) = weak.upgrade() {
                    picker.emit(Picked::Confirm {
                        typed: entry.text().to_string(),
                        selected: picker.selected(),
                    });
                }
            }
        });
        picker.list.connect_row_activated({
            let weak = weak.clone();
            move |_, row| {
                if let Some(picker) = weak.upgrade()
                    && let Some(index) = picker.row_at(row)
                {
                    picker.emit(Picked::Choose(index));
                }
            }
        });
        picker
    }

    pub fn connect_picked(&self, handler: impl Fn(Picked) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    pub fn connect_field_changed(&self, handler: impl Fn(&str) + 'static) {
        self.entry
            .connect_changed(move |entry| handler(entry.text().as_str()));
    }

    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.popover.connect_closed(move |_| handler());
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.resolver
            .replace(Resolver::from_commands_for(keymap, Frontend::Focus).0);
        self.show_rows();
    }

    pub fn set_target(&self, target: &str) {
        self.target.set_text(target);
    }

    pub fn set_field_hint(&self, hint: &str) {
        self.field_hint.set_text(hint);
    }

    pub fn set_footnote(&self, footnote: &str) {
        self.footnote.set_text(footnote);
    }

    pub fn set_rows(&self, rows: Vec<Row>) {
        self.rows.replace(rows);
        self.show_rows();
        // A popup is sized when presented: rows arriving after it opened would scroll inside the
        // empty height.
        if self.popover.is_visible() {
            self.popover.present();
        }
    }

    /// `rect` is in `parent`'s coordinates; `None` is the whole of `parent`.
    pub fn open(&self, parent: &impl IsA<gtk::Widget>, rect: Option<&gdk::Rectangle>) {
        let parent = parent.as_ref();
        if self.popover.parent().as_ref() != Some(parent) {
            if self.popover.parent().is_some() {
                self.popover.unparent();
            }
            self.popover.set_parent(parent);
            // The popover goes with its parent, before GTK finalizes the parent with a child still
            // attached.
            parent.connect_destroy({
                let popover = self.popover.downgrade();
                move |_| {
                    if let Some(popover) = popover.upgrade() {
                        popover.unparent();
                    }
                }
            });
        }
        self.popover.set_pointing_to(rect);
        self.before.replace(
            parent
                .root()
                .and_then(|root| root.focus())
                .filter(|focus| !focus.is_ancestor(&self.popover))
                .map(|focus| focus.downgrade()),
        );
        self.watch_outside(parent);
        self.entry.set_text("");
        self.popover.popup();
        match self.field {
            Field::Filter(_) => {
                self.entry.grab_focus();
            }
            Field::Date => {
                self.list.grab_focus();
                if let Some(row) = self.list.selected_row() {
                    row.grab_focus();
                }
            }
        }
    }

    /// The popup is its own surface, so a press the window sees is outside it.
    fn watch_outside(&self, parent: &gtk::Widget) {
        let Some(root) = parent.root().map(|root| root.upcast::<gtk::Widget>()) else {
            return;
        };
        let watching = self
            .outside
            .borrow()
            .as_ref()
            .and_then(|(window, _)| window.upgrade())
            .is_some_and(|window| window == root);
        if watching {
            return;
        }
        if let Some((window, gesture)) = self.outside.take()
            && let Some(window) = window.upgrade()
        {
            window.remove_controller(&gesture);
        }
        let gesture = gtk::GestureClick::new();
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        gesture.connect_pressed({
            let popover = self.popover.downgrade();
            move |_, _, _, _| {
                if let Some(popover) = popover.upgrade().filter(|popover| popover.is_visible()) {
                    popover.popdown();
                }
            }
        });
        root.add_controller(gesture.clone());
        self.outside.replace(Some((root.downgrade(), gesture)));
    }

    pub fn close(&self) {
        self.popover.popdown();
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// Up and drawn: what a caller waits for before reading it.
    pub fn is_shown(&self) -> bool {
        self.popover
            .child()
            .is_some_and(|child| child.is_mapped() && child.width() > 0)
    }

    pub fn entry(&self) -> &gtk::Entry {
        &self.entry
    }

    /// The text is what takes focus inside an entry, so check it as well as the entry.
    pub fn field_has_keyboard(&self) -> bool {
        self.entry
            .root()
            .and_then(|root| gtk::prelude::RootExt::focus(&root))
            .is_some_and(|focus| {
                focus == *self.entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.entry)
            })
    }

    pub fn popover(&self) -> &gtk::Popover {
        &self.popover
    }

    pub fn selected(&self) -> Option<usize> {
        self.list.selected_row().and_then(|row| self.row_at(&row))
    }

    pub fn select(&self, index: usize) {
        let at = self
            .shown
            .borrow()
            .iter()
            .position(|shown| *shown == Some(index));
        if let Some(row) = at
            .and_then(|at| i32::try_from(at).ok())
            .and_then(|at| self.list.row_at_index(at))
        {
            self.list.select_row(Some(&row));
        }
    }

    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack: Vec<gtk::Widget> = self.popover.child().into_iter().collect();
        while let Some(widget) = stack.pop() {
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && label.is_visible()
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

    /// In the field typing wins: a digit or space in a non-empty filter is text; in an empty one
    /// `1` and `Space` are the picker's.
    pub fn press(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        match key {
            gdk::Key::Down => return self.step(1),
            gdk::Key::Up => return self.step(-1),
            _ => {}
        }
        let Some(chord) = crate::keys::chord(key, state) else {
            return false;
        };
        let in_field = self.field_has_keyboard();
        let bare = state.difference(gdk::ModifierType::SHIFT_MASK).is_empty();
        let empty_filter = matches!(self.field, Field::Filter(_)) && self.entry.text().is_empty();
        let digit_or_space = key
            .to_unicode()
            .is_some_and(|c| c.is_ascii_digit() || c == ' ');
        let typing = postio_ui::pickers::is_typing(in_field, empty_filter, bare, digit_or_space);
        let outcome =
            self.resolver
                .borrow_mut()
                .press(&chord, KeyContext::Picker, typing, Instant::now());
        let Outcome::Command(id) = outcome else {
            return false;
        };
        let Ok(command) = id.parse::<CommandId>() else {
            return false;
        };
        self.command(command)
    }

    pub fn command(&self, command: CommandId) -> bool {
        if let Some(number) = CHOOSE.iter().position(|choose| *choose == command) {
            if let Some(index) = self.numbered(number) {
                self.emit(Picked::Choose(index));
            }
            return true;
        }
        match command {
            CommandId::PickerTypeDate => {
                if self.field == Field::Date {
                    self.entry.grab_focus();
                }
                true
            }
            CommandId::PickerToggle => {
                if let Some(index) = self.selected() {
                    self.emit(Picked::Toggle(index));
                }
                true
            }
            CommandId::PickerConfirm => {
                match self.selected() {
                    Some(index) => self.emit(Picked::Choose(index)),
                    None => self.emit(Picked::Confirm {
                        typed: self.entry.text().to_string(),
                        selected: None,
                    }),
                }
                true
            }
            CommandId::Back => {
                self.close();
                true
            }
            _ => false,
        }
    }

    fn numbered(&self, number: usize) -> Option<usize> {
        self.rows
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.numbered)
            .nth(number)
            .map(|(index, _)| index)
    }

    fn step(&self, by: i32) -> bool {
        let rows = self.shown.borrow();
        let current = self.list.selected_row().map_or(-1, |row| row.index());
        let mut at = current;
        loop {
            at += by;
            let Some(slot) = usize::try_from(at).ok().and_then(|at| rows.get(at)) else {
                return true;
            };
            if slot.is_some() {
                if let Some(row) = self.list.row_at_index(at) {
                    self.list.select_row(Some(&row));
                }
                return true;
            }
        }
    }

    /// `None` for a heading.
    fn row_at(&self, row: &gtk::ListBoxRow) -> Option<usize> {
        let at = usize::try_from(row.index()).ok()?;
        self.shown.borrow().get(at).copied().flatten()
    }

    fn emit(&self, picked: Picked) {
        let handler = self.handler.borrow().clone();
        if let Some(handler) = handler {
            handler(picked);
        }
    }

    fn show_rows(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let keymap = self.keymap.borrow();
        let rows = self.rows.borrow();
        let mut shown = Vec::new();
        let mut first = None;
        let mut number = 0;
        for (index, row) in rows.iter().enumerate() {
            if let Some(section) = &row.section {
                let heading = gtk::Label::new(Some(section));
                heading.add_css_class("postio-picker-section");
                heading.set_xalign(0.0);
                let item = gtk::ListBoxRow::new();
                item.set_child(Some(&heading));
                item.set_selectable(false);
                item.set_activatable(false);
                item.set_can_focus(false);
                self.list.append(&item);
                shown.push(None);
            }
            let line = gtk::Box::new(gtk::Orientation::Horizontal, S2);
            line.add_css_class("postio-picker-row");
            if let Some(Mark::Dot(colour)) = &row.mark {
                line.append(&dot(*colour));
            }
            let name = gtk::Label::new(Some(&row.name));
            name.add_css_class("postio-picker-name");
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&name);
            if !row.detail.is_empty() {
                let detail = gtk::Label::new(Some(&row.detail));
                detail.add_css_class("postio-picker-detail");
                line.append(&detail);
            }
            let key = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            key.add_css_class("postio-picker-key");
            if row.numbered {
                if let Some(cap) = CHOOSE
                    .get(number)
                    .and_then(|command| postio_ui::hints::key(&keymap, *command))
                {
                    key.append(&keyhint::cap(&cap));
                }
                number += 1;
            }
            line.append(&key);
            let item = gtk::ListBoxRow::new();
            item.set_child(Some(&line));
            self.list.append(&item);
            first.get_or_insert(item);
            shown.push(Some(index));
        }
        self.shown.replace(shown);
        if let Some(first) = first {
            self.list.select_row(Some(&first));
        }
    }
}

fn dot(colour: gdk::RGBA) -> gtk::DrawingArea {
    let dot = gtk::DrawingArea::new();
    dot.add_css_class("postio-picker-dot");
    dot.set_content_width(12);
    dot.set_content_height(12);
    dot.set_valign(gtk::Align::Center);
    dot.set_accessible_role(gtk::AccessibleRole::Presentation);
    dot.set_draw_func(move |_, cairo, width, height| {
        cairo.set_source_rgba(
            f64::from(colour.red()),
            f64::from(colour.green()),
            f64::from(colour.blue()),
            f64::from(colour.alpha()),
        );
        cairo.arc(
            f64::from(width) / 2.0,
            f64::from(height) / 2.0,
            4.0,
            0.0,
            std::f64::consts::TAU,
        );
        let _ = cairo.fill();
    });
    dot
}
