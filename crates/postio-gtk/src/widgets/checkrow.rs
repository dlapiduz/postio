//! A boolean that is a value in a form, not a switch that acts elsewhere.
//!
//! Postio drew "Show hover action icons on rows", "Block remote images and
//! trackers" and four others as [`gtk::Switch`]es. A switch and a checkbox
//! say different things: a switch reads as *this takes effect somewhere
//! else, possibly in a moment*, a checkbox as *this is a value in the form
//! I am filling in* (#1179). Every boolean in Postio's settings is the
//! second kind — it is a key in `config.toml`, and the file is the form —
//! so every one of them is a checkbox. ADR 0027 has the rule.
//!
//! # Why not a bare `gtk::CheckButton`
//!
//! Because of the guard. Setting a `CheckButton`'s state fires `toggled`,
//! so a pane redrawing itself from a fresh read of the file writes the value
//! straight back — and the old panel's answer was to connect the handler
//! *after* setting the state and rebuild the whole row on every change. That
//! works right up until something needs to update a control it did not just
//! build. [`CheckRow::set_active`] is silent by construction instead.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

type ToggledHandler = Box<dyn Fn(bool)>;

/// One labelled checkbox: square, visibly on or off, keyboard-operable.
pub struct CheckRow {
    check: gtk::CheckButton,
    handlers: Rc<RefCell<Vec<ToggledHandler>>>,
    /// Set while [`CheckRow::set_active`] is moving the box — see the module
    /// doc. Shared with the `toggled` closure, which is the thing that has
    /// to read it.
    setting: Rc<Cell<bool>>,
}

impl CheckRow {
    /// A checkbox labelled `label`.
    pub fn new(label: &str) -> Self {
        let check = gtk::CheckButton::with_label(label);
        check.add_css_class("postio-check");
        check.set_halign(gtk::Align::Start);

        let handlers: Rc<RefCell<Vec<ToggledHandler>>> = Rc::new(RefCell::new(Vec::new()));
        let setting = Rc::new(Cell::new(false));
        check.connect_toggled({
            let handlers = Rc::clone(&handlers);
            let setting = Rc::clone(&setting);
            move |check| {
                // Showing the file's value is not changing it.
                if setting.get() {
                    return;
                }
                let active = check.is_active();
                for handler in handlers.borrow().iter() {
                    handler(active);
                }
            }
        });

        Self {
            check,
            handlers,
            setting,
        }
    }

    /// The widget to append.
    pub fn widget(&self) -> &gtk::CheckButton {
        &self.check
    }

    /// Whether the box is checked.
    pub fn is_active(&self) -> bool {
        self.check.is_active()
    }

    /// Checks or clears the box **without** telling anyone.
    pub fn set_active(&self, active: bool) {
        self.setting.set(true);
        self.check.set_active(active);
        self.setting.set(false);
    }

    /// Runs `handler` with the new value whenever a person changes it.
    pub fn connect_toggled(&self, handler: impl Fn(bool) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }
}
