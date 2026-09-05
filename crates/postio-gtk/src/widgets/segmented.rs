//! A closed set of choices with all of them on screen.
//!
//! `System / Light / Dark`. `Airy / Snug / Compact`. `IMAP IDLE / Every 5 min
//! / Manual`. Postio drew every one of these as a [`gtk::DropDown`], which
//! hides its own vocabulary: a person reading the settings could not see that
//! there were three answers without opening the thing first, and three
//! answers is exactly the case where seeing them all *is* the point (#1179).
//!
//! A dropdown still earns its place for an open or long list — the signature
//! picker has as many entries as the account has signatures, and there is no
//! showing those in a row. The rule is in ADR 0027.
//!
//! # It is a radio group, not a row of buttons
//!
//! Built from grouped [`gtk::ToggleButton`]s, so GTK gives the keyboard
//! behaviour and the accessibility for free: arrow keys move within the
//! group, exactly one member is ever active, and a screen reader announces it
//! as a radio group rather than as three unrelated buttons. The joined,
//! square-cornered look is [`shell.css`]'s job, not this module's.
//!
//! [`shell.css`]: ../../data/shell.css

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

type SelectedHandler = Box<dyn Fn(usize)>;

/// One choice from a handful, all of them visible.
pub struct SegmentedControl {
    root: gtk::Box,
    segments: Vec<gtk::ToggleButton>,
    handlers: Rc<RefCell<Vec<SelectedHandler>>>,
    /// Set while [`SegmentedControl::set_selected`] is moving the active
    /// segment, so redrawing a pane from a fresh read of `config.toml` does
    /// not write back the value it just read.
    ///
    /// Every caller in the old settings panel did this by hand, by setting
    /// the control's state *before* connecting its handler and rebuilding
    /// the whole row whenever the value changed. That works until something
    /// needs to update a control it did not just build, and it is the
    /// reason those rows were rebuilt rather than updated.
    setting: Rc<Cell<bool>>,
}

impl SegmentedControl {
    /// A group of segments labelled `options`, announced as `label`.
    ///
    /// Nothing is selected until [`set_selected`](Self::set_selected) says
    /// so: the value belongs to the file this control is a view of, and
    /// guessing it here would show a value that is not in the file.
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
            // Grouping is what makes this a radio group rather than three
            // toggles that happen to sit together: GTK unsets the previous
            // member itself, and refuses to leave the group empty.
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
                // Showing the file's value is not changing it.
                if setting.get() {
                    return;
                }
                // One press moves the group twice — the old segment off,
                // the new one on — and only the second is the choice. A
                // handler that wrote a config value would otherwise write
                // it twice, the first time with the wrong index.
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

    /// The widget to append.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Which segment is active, or `None` before one has been chosen.
    pub fn selected(&self) -> Option<usize> {
        self.segments.iter().position(|button| button.is_active())
    }

    /// Makes `index` the active segment **without** telling anyone.
    ///
    /// An out-of-range index selects nothing rather than panicking: the
    /// value comes from a file a person can type into, and a `[ui] theme`
    /// nobody recognises is a validation problem for the footer line to
    /// report, not a reason to take the window down.
    pub fn set_selected(&self, index: usize) {
        self.setting.set(true);
        if let Some(button) = self.segments.get(index) {
            button.set_active(true);
        }
        self.setting.set(false);
    }

    /// Presses segment `index` the way a pointer would, for a test.
    pub fn test_press(&self, index: usize) {
        if let Some(button) = self.segments.get(index) {
            button.set_active(true);
        }
    }

    /// Runs `handler` with the index a person just chose.
    ///
    /// Never fires for [`set_selected`](Self::set_selected), and never fires
    /// for the segment being *deactivated* — one press moves the group once,
    /// so a handler that wrote a config value would otherwise write it twice.
    pub fn connect_selected(&self, handler: impl Fn(usize) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }
}
