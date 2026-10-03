//! The pickers (specs/007-postio-focus US5; contracts/focus-surface.md,
//! "Pickers"; screens 11-14): a 380 px popover anchored to the focused
//! row, titled with what it does and what it acts on, holding a short list
//! and a field, with a footnote saying what happens next.
//!
//! [`Picker`] is the frame all four share: the title row, the list with its
//! number keys and section headings, the field -- a typed date for snooze
//! and remind, a filter for label and move -- and the footnote. It reads
//! its keys through the one keymap in `Context::Picker`, so `1`-`4`,
//! `Tab`, `Space` and `Enter` are the keymap's, not this file's. It
//! decides nothing: what a choice means is the picker built on it
//! ([`WhenPicker`], and Focus's label and move pickers).
//!
//! The words and times are `postio_ui::pickers`', where a test proves them
//! without a display.

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

/// The popover's width (contracts/focus-surface.md).
pub const WIDTH: i32 = 380;

/// The number keys, in order: the first four keyed rows answer to them.
const CHOOSE: [CommandId; 4] = [
    CommandId::PickerChoose1,
    CommandId::PickerChoose2,
    CommandId::PickerChoose3,
    CommandId::PickerChoose4,
];

/// A row's mark before its name.
#[derive(Debug, Clone)]
pub enum Mark {
    /// A label's colour dot.
    Dot(gdk::RGBA),
}

/// One row of a picker's list.
#[derive(Debug, Clone, Default)]
pub struct Row {
    /// The heading of the section this row starts, when it starts one:
    /// "Recent", "All folders".
    pub section: Option<String>,
    /// What sits before the name.
    pub mark: Option<Mark>,
    /// The name, in bold.
    pub name: String,
    /// On the right: a time, a count, "✓ applied".
    pub detail: String,
    /// Whether a number key chooses it: the first four such rows answer to
    /// `1`-`4`, in order.
    pub numbered: bool,
}

/// Which field a picker holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// A date typed in words, under the list; `Tab` reaches it.
    Date,
    /// A filter over the list, above it, holding the keyboard from the
    /// start; its placeholder says what it filters.
    Filter(&'static str),
}

/// What a person did in a picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// A row was chosen: by its number key, a click, or `Enter` on it.
    Choose(usize),
    /// `Enter` in the field: what was typed, and the row selected then.
    Confirm {
        /// The field's text.
        typed: String,
        /// The row selected, if any.
        selected: Option<usize>,
    },
    /// `Space` on a row.
    Toggle(usize),
}

type Handler = Rc<dyn Fn(Picked)>;

/// The frame a picker is drawn in. See the module.
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
    /// Every row, as last set.
    rows: RefCell<Vec<Row>>,
    /// What each row of the list is: a heading, or the row at that index.
    shown: RefCell<Vec<Option<usize>>>,
    handler: RefCell<Option<Handler>>,
    /// The press-anywhere-else watcher on the window the picker last
    /// opened in.
    outside: RefCell<Option<(glib::WeakRef<gtk::Widget>, gtk::GestureClick)>>,
}

impl Picker {
    /// A closed picker titled `title`, holding `field`, with `footnote`
    /// under it, its keys read from `keymap`.
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
        // No grab: a grabbing popup is the compositor's to dismiss, and one
        // opened by a key rather than a click is dismissed at once where
        // nothing has given the window a pointer serial -- which is every
        // headless run, and a real session now and then. The picker holds
        // the keyboard through the window's root focus instead, and a press
        // outside it closes it (see `open`).
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

    /// Run `handler` with what the person does.
    pub fn connect_picked(&self, handler: impl Fn(Picked) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Run `handler` whenever the field's text changes: a filter re-lists.
    pub fn connect_field_changed(&self, handler: impl Fn(&str) + 'static) {
        self.entry
            .connect_changed(move |entry| handler(entry.text().as_str()));
    }

    /// Run `handler` when the picker closes, however it closes.
    pub fn connect_closed(&self, handler: impl Fn() + 'static) {
        self.popover.connect_closed(move |_| handler());
    }

    /// Read every key from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.keymap.replace(keymap.clone());
        self.resolver
            .replace(Resolver::from_commands_for(keymap, Frontend::Focus).0);
        self.show_rows();
    }

    /// Say what the picker acts on: "Ada Moreno · Atlas Q3 budget".
    pub fn set_target(&self, target: &str) {
        self.target.set_text(target);
    }

    /// Set the line under the date field.
    pub fn set_field_hint(&self, hint: &str) {
        self.field_hint.set_text(hint);
    }

    /// Set the footnote.
    pub fn set_footnote(&self, footnote: &str) {
        self.footnote.set_text(footnote);
    }

    /// List `rows`, and select the first.
    pub fn set_rows(&self, rows: Vec<Row>) {
        self.rows.replace(rows);
        self.show_rows();
        // A popup is sized when it is presented: rows that land after it
        // opened -- a label picker's, read from the store -- would scroll
        // inside the height it had when it was empty.
        if self.popover.is_visible() {
            self.popover.present();
        }
    }

    /// Open the picker from `parent`, pointing at `rect` in its coordinates
    /// (the whole of `parent` when `None`), with the field empty.
    pub fn open(&self, parent: &impl IsA<gtk::Widget>, rect: Option<&gdk::Rectangle>) {
        let parent = parent.as_ref();
        if self.popover.parent().as_ref() != Some(parent) {
            if self.popover.parent().is_some() {
                self.popover.unparent();
            }
            self.popover.set_parent(parent);
            // The popover goes with what it hangs from, before GTK would
            // finalize that with a child still attached.
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

    /// Close the picker on a press in the window it opened over: the popup
    /// is its own surface, so a press the window sees is outside it.
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

    /// Close the picker, changing nothing.
    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Whether the picker is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// Whether the picker is up and drawn: what a caller waits for before
    /// reading it.
    pub fn is_shown(&self) -> bool {
        self.popover
            .child()
            .is_some_and(|child| child.is_mapped() && child.width() > 0)
    }

    /// The field: the typed date, or the filter.
    pub fn entry(&self) -> &gtk::Entry {
        &self.entry
    }

    /// Whether the keyboard is in the field: its text, which is what takes
    /// the focus inside an entry, or the entry itself.
    pub fn field_has_keyboard(&self) -> bool {
        self.entry
            .root()
            .and_then(|root| gtk::prelude::RootExt::focus(&root))
            .is_some_and(|focus| {
                focus == *self.entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.entry)
            })
    }

    /// The popover, for a caller that places or styles it.
    pub fn popover(&self) -> &gtk::Popover {
        &self.popover
    }

    /// The index of the selected row, if any.
    pub fn selected(&self) -> Option<usize> {
        self.list.selected_row().and_then(|row| self.row_at(&row))
    }

    /// Select the `index`th row.
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

    /// Every text the picker shows, in order: what a test reads.
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

    /// Handle one key, as the picker's key controller does; `true` when the
    /// picker used it.
    ///
    /// The keymap's `Context::Picker` decides: `1`-`4` choose, `Tab` goes
    /// to the date field, `Space` toggles, `Enter` confirms and `Escape`
    /// closes. In the field, typing wins, as everywhere: a digit or a space
    /// typed into a filter is text -- except into an empty filter, where
    /// there is nothing to type into yet and `1` and `Space` are the
    /// picker's.
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

    /// Run a picker command; `false` for one a picker does not answer.
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

    /// The row the `number`th number key chooses.
    fn numbered(&self, number: usize) -> Option<usize> {
        self.rows
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, row)| row.numbered)
            .nth(number)
            .map(|(index, _)| index)
    }

    /// Move the selection `by` rows, over the headings.
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

    /// The index in `rows` of the list row `row`; `None` for a heading.
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

    /// Draw the rows, with their headings, marks, details and number keys.
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

/// A label's colour dot.
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
