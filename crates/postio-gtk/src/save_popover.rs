//! The Save popover (spec 010 step 6, design §3.9, screen 12): the
//! controller's [`SaveView`] over a search's results, hung from their Save
//! search button.
//!
//! Every word is the controller's, as on the Mac
//! (`macos/Sources/PostioKit/SavePopoverView.swift`): the title, the name it
//! offers, the query's terms as read-only chips, and three switches as they
//! start. Save ↩ says what is set back as [`Input::SaveSearchAs`], once;
//! Cancel, Escape or a click away write nothing, and the controller is not
//! told, as the Mac's `SavePopoverModel` does not tell it.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_focus::{Input, SaveView};
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};

/// The Save popover.
pub struct SavePopover {
    popover: gtk::Popover,
    title: gtk::Label,
    name_label: gtk::Label,
    name: gtk::Entry,
    chips: gtk::FlowBox,
    pin: Switch,
    notify: Switch,
    rolling: Switch,
    cancel: gtk::Button,
    save: gtk::Button,
    /// Save was said for the view shown: a second Return before the
    /// controller takes it down says nothing more.
    sent: Cell<bool>,
    say: Rc<dyn Fn(Input)>,
}

/// One switch with its words: the title over its note, the switch on the
/// right.
struct Switch {
    row: gtk::Box,
    title: gtk::Label,
    note: gtk::Box,
    switch: gtk::Switch,
}

impl Switch {
    fn new() -> Switch {
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.add_css_class("focus-save-switch-title");
        let note = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        note.add_css_class("focus-save-switch-note");
        let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
        words.set_hexpand(true);
        words.append(&title);
        words.append(&note);
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        row.add_css_class("focus-save-switch");
        row.append(&words);
        row.append(&switch);
        Switch {
            row,
            title,
            note,
            switch,
        }
    }

    /// Say `title`, then `note` and `key` under it, and start `on`.
    fn show(&self, title: &str, note: &str, key: Option<&str>, on: bool) {
        self.title.set_text(title);
        self.switch
            .update_property(&[gtk::accessible::Property::Label(title)]);
        while let Some(child) = self.note.first_child() {
            self.note.remove(&child);
        }
        let words = gtk::Label::new(Some(note));
        words.add_css_class("dim-label");
        words.set_xalign(0.0);
        words.set_wrap(true);
        self.note.append(&words);
        if let Some(key) = key {
            self.note.append(&keyhint::cap(key));
        }
        self.switch.set_active(on);
    }
}

impl SavePopover {
    /// A popover nothing has opened; `say` hands the controller what Save
    /// says.
    pub fn new(say: impl Fn(Input) + 'static) -> Rc<Self> {
        let title = gtk::Label::new(None);
        title.add_css_class("focus-save-title");
        title.set_xalign(0.0);
        let name_label = gtk::Label::new(None);
        name_label.add_css_class("dim-label");
        name_label.set_xalign(0.0);
        let name = gtk::Entry::new();
        name.add_css_class("focus-save-name");
        // The terms wrap within the popover's width rather than widen it.
        let chips = gtk::FlowBox::new();
        chips.add_css_class("focus-save-chips");
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_column_spacing(S1 as u32);
        chips.set_row_spacing(S1 as u32);
        chips.set_max_children_per_line(4);
        let pin = Switch::new();
        let notify = Switch::new();
        let rolling = Switch::new();
        let cancel = gtk::Button::new();
        let save = gtk::Button::new();
        save.add_css_class("focus-save-button");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, S2);
        buttons.set_halign(gtk::Align::End);
        buttons.append(&cancel);
        buttons.append(&save);

        let content = gtk::Box::new(gtk::Orientation::Vertical, S2);
        content.add_css_class("focus-save");
        content.append(&title);
        content.append(&name_label);
        content.append(&name);
        content.append(&chips);
        content.append(&pin.row);
        content.append(&notify.row);
        content.append(&rolling.row);
        content.append(&buttons);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&content));
        popover.set_position(gtk::PositionType::Bottom);
        // From its button's end: the button sits at the window's right
        // edge, and a compositor that does not slide a popup back inside
        // would draw it cut off (as the filter popovers open from their
        // buttons' start).
        popover.set_halign(gtk::Align::End);
        popover.set_has_arrow(false);
        popover.add_css_class("focus-save-popover");

        let pop = Rc::new(SavePopover {
            popover,
            title,
            name_label,
            name,
            chips,
            pin,
            notify,
            rolling,
            cancel,
            save,
            sent: Cell::new(false),
            say: Rc::new(say),
        });
        let weak = Rc::downgrade(&pop);
        pop.name.connect_activate({
            let weak = weak.clone();
            move |_| {
                if let Some(pop) = weak.upgrade() {
                    pop.say_save();
                }
            }
        });
        pop.save.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(pop) = weak.upgrade() {
                    pop.say_save();
                }
            }
        });
        pop.cancel.connect_clicked(move |_| {
            if let Some(pop) = weak.upgrade() {
                pop.close();
            }
        });
        pop
    }

    /// Draw `view`, hung from `anchor`, the keyboard in the Name field
    /// with the name offered selected, so typing replaces it; `cancel_key`
    /// is Back's, which Cancel is.
    pub fn show(&self, view: &SaveView, anchor: &impl IsA<gtk::Widget>, cancel_key: Option<&str>) {
        if self.popover.parent().as_ref() != Some(anchor.upcast_ref()) {
            if self.popover.parent().is_some() {
                self.popover.unparent();
            }
            self.popover.set_parent(anchor);
        }
        self.sent.set(false);
        self.title.set_text(&view.title);
        self.name_label.set_text(&view.name_label);
        self.name.set_text(&view.name);
        self.chips.remove_all();
        for chip in &view.chips {
            let label = gtk::Label::new(Some(chip));
            label.add_css_class("focus-save-chip");
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            self.chips.append(&label);
        }
        self.pin.show(
            &view.pin_label,
            &view.pin_note,
            view.pin_key.as_deref(),
            view.pin,
        );
        self.notify
            .show(&view.notify_label, &view.notify_note, None, view.notify);
        // Drawn only when the query has a date to keep (§3.9).
        self.rolling.row.set_visible(view.rolling_note.is_some());
        if let Some(note) = &view.rolling_note {
            self.rolling
                .show(&view.rolling_label, note, None, view.rolling);
        }
        keyhint::dress(&self.cancel, &view.cancel, cancel_key);
        keyhint::dress(&self.save, &view.save, view.save_key.as_deref());
        self.popover.popup();
        self.name.grab_focus();
        self.name.select_region(0, -1);
    }

    /// Take it down: the controller's `SavePopover(None)`, Cancel, or Esc.
    /// Nothing is written.
    pub fn close(&self) {
        self.popover.popdown();
    }

    /// Save ↩: the controller writes it, and takes the popover down.
    fn say_save(&self) {
        if !self.is_open() || self.sent.replace(true) {
            return;
        }
        (self.say)(Input::SaveSearchAs {
            name: self.name.text().to_string(),
            pin: self.pin.switch.is_active(),
            notify: self.notify.switch.is_active(),
            rolling: self.rolling.row.is_visible() && self.rolling.switch.is_active(),
        });
    }

    /// Run `closed` whenever it goes, however it went: GTK hands the
    /// keyboard back to the button it hangs from, which is not where it
    /// was.
    pub fn connect_closed(&self, closed: impl Fn() + 'static) {
        self.popover.connect_closed(move |_| closed());
    }

    /// Whether it is up.
    pub fn is_open(&self) -> bool {
        self.popover.is_visible()
    }

    /// What the Name field holds.
    pub fn name(&self) -> String {
        self.name.text().to_string()
    }

    /// Whether the keyboard is in the Name field.
    pub fn name_has_keyboard(&self) -> bool {
        self.name.has_focus() || self.name.delegate().is_some_and(|text| text.has_focus())
    }

    /// Every word it says, top to bottom.
    pub fn texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.popover.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if !widget.is_visible() {
                continue;
            }
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                said.push(label.text().to_string());
            }
            let mut children = Vec::new();
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                children.push(next);
            }
            stack.extend(children.into_iter().rev());
        }
        said
    }

    /// A key, as the popover's own field and buttons take it: Return saves
    /// and Escape cancels. Whether it was the popover's -- what a test
    /// presses, since a popover's keys never pass the window's.
    pub fn key(&self, key: gtk::gdk::Key) -> bool {
        if !self.is_open() {
            return false;
        }
        match key {
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => self.say_save(),
            gtk::gdk::Key::Escape => self.close(),
            _ => return false,
        }
        true
    }
}
