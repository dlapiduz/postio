use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_focus::Input;
use postio_model::calendar::EditScope;
use postio_ui::calendar::editor::{self as words, EditorField, EditorView};
use postio_widgets::widgets::space::{S2, S3};

use crate::list::feed::Feed;

const SCOPES: [EditScope; 3] = [EditScope::This, EditScope::ThisAndFollowing, EditScope::All];

pub struct EventEditor {
    card: gtk::Box,
    heading: gtk::Label,
    entries: HashMap<EditorField, gtk::Entry>,
    labels: HashMap<EditorField, gtk::Widget>,
    notes: gtk::TextView,
    when: gtk::Label,
    error: gtk::Label,
    scope_row: gtk::Box,
    scopes: Vec<gtk::ToggleButton>,
    keys: gtk::Box,
    echo: Rc<Cell<bool>>,
    shown: RefCell<Option<EditorView>>,
}

impl EventEditor {
    pub fn new(feed: &Feed, act: Rc<dyn Fn(CommandId)>) -> Rc<Self> {
        let card = gtk::Box::new(gtk::Orientation::Vertical, S2);
        card.add_css_class("focus-calendar-card");
        card.add_css_class("focus-calendar-editor");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_visible(false);
        card.set_accessible_role(gtk::AccessibleRole::Dialog);

        let heading = gtk::Label::new(None);
        heading.add_css_class("focus-calendar-card-title");
        heading.set_xalign(0.0);
        heading.set_hexpand(true);
        let close = postio_widgets::widgets::button::close_button();
        let back = Rc::clone(&act);
        close.connect_clicked(move |_| back(CommandId::Back));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        header.append(&heading);
        header.append(&close);
        card.append(&header);

        let echo = Rc::new(Cell::new(false));
        let grid = gtk::Grid::new();
        grid.set_row_spacing(S2 as u32);
        grid.set_column_spacing(S3 as u32);
        let mut entries = HashMap::new();
        let mut labels: HashMap<EditorField, gtk::Widget> = HashMap::new();
        let mut row = 0;
        for field in EditorField::ALL {
            let label = gtk::Label::new(Some(field.label()));
            label.set_xalign(1.0);
            label.add_css_class("dim-label");
            if field == EditorField::Notes {
                continue;
            }
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some(field.placeholder()));
            entry.set_hexpand(true);
            entry.set_width_chars(36);
            label.set_mnemonic_widget(Some(&entry));
            entry.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
            let (feed, echo) = (feed.clone(), Rc::clone(&echo));
            entry.connect_changed(move |entry| {
                if !echo.get() {
                    feed.input(Input::EventEditorTyped {
                        field,
                        text: entry.text().to_string(),
                    });
                }
            });
            let confirm = Rc::clone(&act);
            entry.connect_activate(move |_| confirm(CommandId::SaveEvent));
            grid.attach(&label, 0, row, 1, 1);
            grid.attach(&entry, 1, row, 1, 1);
            labels.insert(field, label.upcast::<gtk::Widget>());
            entries.insert(field, entry);
            row += 1;
            if field == EditorField::Ends {
                row += 1;
            }
        }
        let when = gtk::Label::new(None);
        when.set_xalign(0.0);
        when.add_css_class("focus-calendar-card-line");
        grid.attach(&when, 1, 3, 1, 1);
        card.append(&grid);

        let notes_label = gtk::Label::new(Some(EditorField::Notes.label()));
        notes_label.set_xalign(0.0);
        notes_label.add_css_class("dim-label");
        let notes = gtk::TextView::new();
        notes.set_wrap_mode(gtk::WrapMode::WordChar);
        notes.add_css_class("focus-calendar-editor-notes");
        notes.set_size_request(-1, 72);
        notes.update_relation(&[gtk::accessible::Relation::LabelledBy(&[
            notes_label.upcast_ref()
        ])]);
        {
            let (feed, echo) = (feed.clone(), Rc::clone(&echo));
            notes.buffer().connect_changed(move |buffer| {
                if !echo.get() {
                    feed.input(Input::EventEditorTyped {
                        field: EditorField::Notes,
                        text: buffer
                            .text(&buffer.start_iter(), &buffer.end_iter(), false)
                            .to_string(),
                    });
                }
            });
        }
        labels.insert(EditorField::Notes, notes_label.clone().upcast());
        card.append(&notes_label);
        card.append(&notes);

        let scope_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        scope_row.add_css_class("linked");
        let mut scopes: Vec<gtk::ToggleButton> = Vec::new();
        for scope in SCOPES {
            let button = gtk::ToggleButton::with_label(words::scope_label(scope));
            if let Some(first) = scopes.first() {
                button.set_group(Some(first));
            }
            let (feed, echo) = (feed.clone(), Rc::clone(&echo));
            button.connect_toggled(move |button| {
                if button.is_active() && !echo.get() {
                    feed.input(Input::EventEditorScope(scope));
                }
            });
            scope_row.append(&button);
            scopes.push(button);
        }
        card.append(&scope_row);

        let error = gtk::Label::new(None);
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.add_css_class("error");
        error.set_accessible_role(gtk::AccessibleRole::Alert);
        card.append(&error);

        let keys = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        keys.set_margin_top(S2);
        card.append(&keys);

        notes.set_accepts_tab(false);
        let order: Vec<gtk::Widget> = [
            EditorField::Title,
            EditorField::Starts,
            EditorField::Ends,
            EditorField::Location,
            EditorField::People,
        ]
        .iter()
        .filter_map(|field| entries.get(field).map(|entry| entry.clone().upcast()))
        .chain(std::iter::once(notes.clone().upcast()))
        .collect();
        let tab_for_keys_gtk_window_bindings_miss = gtk::EventControllerKey::new();
        tab_for_keys_gtk_window_bindings_miss.connect_key_pressed(move |_, key, _, state| {
            let back = match key {
                gtk::gdk::Key::Tab if state.is_empty() => false,
                gtk::gdk::Key::ISO_Left_Tab => true,
                gtk::gdk::Key::Tab if state == gtk::gdk::ModifierType::SHIFT_MASK => true,
                _ => return gtk::glib::Propagation::Proceed,
            };
            let at = order
                .iter()
                .position(|widget| widget.has_focus() || widget.focus_child().is_some());
            let next = match (at, back) {
                (Some(at), false) => (at + 1) % order.len(),
                (Some(at), true) => (at + order.len() - 1) % order.len(),
                (None, _) => 0,
            };
            order[next].grab_focus();
            gtk::glib::Propagation::Stop
        });
        card.add_controller(tab_for_keys_gtk_window_bindings_miss);

        Rc::new(EventEditor {
            card,
            heading,
            entries,
            labels,
            notes,
            when,
            error,
            scope_row,
            scopes,
            keys,
            echo,
            shown: RefCell::new(None),
        })
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.card
    }

    pub fn is_open(&self) -> bool {
        self.shown.borrow().is_some()
    }

    pub fn entry(&self, field: EditorField) -> Option<&gtk::Entry> {
        self.entries.get(&field)
    }

    pub fn shown(&self) -> Option<EditorView> {
        self.shown.borrow().clone()
    }

    pub fn show(&self, view: Option<&EditorView>) {
        if self.shown.borrow().as_ref() == view {
            return;
        }
        let opening = self.shown.borrow().is_none() && view.is_some();
        self.shown.replace(view.cloned());
        let Some(view) = view else {
            self.card.set_visible(false);
            return;
        };
        self.echo.set(true);
        self.heading.set_text(&view.heading);
        self.card
            .update_property(&[gtk::accessible::Property::Label(&view.heading)]);
        let shown = |field: EditorField| view.fields.iter().any(|(each, _)| *each == field);
        for field in EditorField::ALL {
            if let Some(label) = self.labels.get(&field) {
                label.set_visible(shown(field));
            }
            if let Some(entry) = self.entries.get(&field) {
                entry.set_visible(shown(field));
            }
        }
        self.notes.set_visible(shown(EditorField::Notes));
        self.when.set_visible(shown(EditorField::Starts));
        for (field, text) in &view.fields {
            if *field == EditorField::Notes {
                let buffer = self.notes.buffer();
                let held = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                if held.as_str() != text {
                    buffer.set_text(text);
                }
            } else if let Some(entry) = self.entries.get(field)
                && entry.text().as_str() != text
            {
                entry.set_text(text);
                entry.set_position(-1);
            }
        }
        match &view.when {
            Ok(line) => {
                self.when.set_text(line);
                self.when.remove_css_class("error");
            }
            Err(why) => {
                self.when.set_text(why);
                self.when.add_css_class("error");
            }
        }
        self.scope_row.set_visible(view.scope.is_some());
        if let Some(scope) = view.scope {
            for (button, each) in self.scopes.iter().zip(SCOPES) {
                button.set_active(each == scope);
            }
        }
        self.error.set_visible(view.error.is_some());
        self.error
            .set_text(view.error.as_deref().unwrap_or_default());
        while let Some(child) = self.keys.first_child() {
            self.keys.remove(&child);
        }
        for hint in &view.keys {
            self.keys.append(&postio_widgets::widgets::keyhint::chip(
                hint,
                "focus-calendar-card-key",
            ));
        }
        self.echo.set(false);
        self.card.set_visible(true);
        if opening {
            match view
                .fields
                .first()
                .and_then(|(field, _)| self.entries.get(field))
            {
                Some(first) => {
                    first.grab_focus();
                }
                None => {
                    self.card.set_focusable(true);
                    self.card.grab_focus();
                }
            }
        }
    }
}
