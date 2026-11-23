//! A closed set of choices with all of them on screen (ADR 0027), built from grouped
//! `ToggleButton`s so GTK gives radio-group keyboard behaviour and accessibility.
//! The joined look is `shell.css`'s job.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

type SelectedHandler = Box<dyn Fn(usize)>;

pub struct SegmentedControl {
    root: gtk::Box,
    segments: Vec<gtk::ToggleButton>,
    handlers: Rc<RefCell<Vec<SelectedHandler>>>,
    /// Set while `set_selected` moves the segment, so redrawing from a fresh read of `config.toml`
    /// does not write the value back.
    setting: Rc<Cell<bool>>,
}

impl SegmentedControl {
    /// Nothing is selected until `set_selected`: the value belongs to the file, and guessing would
    /// show one that is not in it.
    pub fn new(label: &str, options: &[&str]) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("postio-segmented");
        root.set_halign(gtk::Align::Start);
        root.update_property(&[gtk::accessible::Property::Label(label)]);
        root.set_accessible_role(gtk::AccessibleRole::RadioGroup);

        let mut segments = Vec::with_capacity(options.len());
        for (index, option) in options.iter().enumerate() {
            let button = gtk::ToggleButton::with_label(option);
            button.add_css_class("postio-segment");
            if index == 0 {
                button.add_css_class("postio-segment-first");
            }
            if index + 1 == options.len() {
                button.add_css_class("postio-segment-last");
            }
            // Grouping makes it a radio group: GTK unsets the previous member and refuses to leave
            // the group empty.
            if let Some(first) = segments.first() {
                button.set_group(Some(first));
            }
            root.append(&button);
            segments.push(button);
        }

        let handlers: Rc<RefCell<Vec<SelectedHandler>>> = Rc::new(RefCell::new(Vec::new()));
        let setting = Rc::new(Cell::new(false));
        for (index, button) in segments.iter().enumerate() {
            let handlers = Rc::clone(&handlers);
            let setting = Rc::clone(&setting);
            button.connect_toggled(move |button| {
                if setting.get() {
                    return;
                }
                // One press moves the group twice, old off then new on; only the second is the
                // choice.
                if !button.is_active() {
                    return;
                }
                for handler in handlers.borrow().iter() {
                    handler(index);
                }
            });
        }

        Self {
            root,
            segments,
            handlers,
            setting,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn selected(&self) -> Option<usize> {
        self.segments.iter().position(|button| button.is_active())
    }

    /// An out-of-range index selects nothing rather than panicking: the value comes from a file a
    /// person edits.
    pub fn set_selected(&self, index: usize) {
        self.setting.set(true);
        if let Some(button) = self.segments.get(index) {
            button.set_active(true);
        }
        self.setting.set(false);
    }

    pub fn test_press(&self, index: usize) {
        if let Some(button) = self.segments.get(index) {
            button.set_active(true);
        }
    }

    /// Never fires for `set_selected`, nor for the segment being deactivated: one press moves the
    /// group twice.
    pub fn connect_selected(&self, handler: impl Fn(usize) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }
}
