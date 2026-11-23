//! A boolean that is a value in a form: a checkbox, not a switch (ADR 0027).
//! `CheckRow::set_active` is silent by construction: setting a `CheckButton` fires `toggled`, which
//! would write a redrawn value back to the file.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

type ToggledHandler = Box<dyn Fn(bool)>;

pub struct CheckRow {
    check: gtk::CheckButton,
    handlers: Rc<RefCell<Vec<ToggledHandler>>>,
    /// Set while `set_active` moves the box; the `toggled` closure reads it.
    setting: Rc<Cell<bool>>,
}

impl CheckRow {
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

    pub fn widget(&self) -> &gtk::CheckButton {
        &self.check
    }

    pub fn is_active(&self) -> bool {
        self.check.is_active()
    }

    /// Checks or clears the box **without** telling anyone.
    pub fn set_active(&self, active: bool) {
        self.setting.set(true);
        self.check.set_active(active);
        self.setting.set(false);
    }

    pub fn connect_toggled(&self, handler: impl Fn(bool) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }
}
