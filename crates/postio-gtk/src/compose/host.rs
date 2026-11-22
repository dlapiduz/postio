//! Focus's compose dialog as a `ComposerHost`: taking the pane presents the dialog; giving it back
//! closes it and returns
//! the keyboard to the row it left from.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use postio_core::{CommandId, Keymap};
use postio_ui::focus_dialog;
use postio_ui::keymap::KeyContext;
use postio_widgets::composer::{Composer, ComposerHost};

use crate::window::FocusWindow;

pub const WINDOW: (i32, i32) = (1440, 900);

type CommandHandler = Box<dyn Fn(CommandId)>;

pub struct DialogHost {
    pub(super) window: glib::WeakRef<FocusWindow>,
    pub(super) dialog: adw::Dialog,
    pub(super) slot: gtk::Box,
    column: adw::Clamp,
    following: Cell<bool>,
    commands: RefCell<Vec<CommandHandler>>,
    showing: Cell<bool>,
    surface: gtk::Widget,
    /// Shared with the resize follower, which fits the column to whichever host holds it.
    pane: Rc<RefCell<Option<gtk::Box>>>,
    /// Neither the dialog nor the pane holds the surface while it is detached in a window of its
    /// own.
    parked: Cell<bool>,
}

impl DialogHost {
    pub fn new(
        window: &FocusWindow,
        dialog: adw::Dialog,
        slot: gtk::Box,
        column: adw::Clamp,
        surface: gtk::Widget,
    ) -> Self {
        DialogHost {
            window: window.downgrade(),
            dialog,
            slot,
            column,
            following: Cell::new(false),
            commands: RefCell::default(),
            showing: Cell::new(false),
            surface,
            pane: Rc::default(),
            parked: Cell::new(false),
        }
    }

    pub fn in_pane(&self) -> bool {
        self.pane.borrow().is_some()
    }

    /// An open composer stays open and is drawn where it now is.
    pub fn place(&self, slot: Option<&gtk::Box>) {
        let here = self.pane.borrow().clone();
        if self.parked.get() {
            self.pane.replace(slot.cloned());
            return;
        }
        match (slot, here) {
            (Some(slot), Some(here)) if *slot == here => return,
            (None, None) => return,
            (Some(slot), here) => {
                match &here {
                    None => {
                        if self.showing.get() {
                            self.dialog.force_close();
                        }
                        self.dialog.set_child(None::<&gtk::Widget>);
                    }
                    Some(here) => here.remove(&self.surface),
                }
                slot.append(&self.surface);
                self.pane.replace(Some(slot.clone()));
            }
            (None, Some(here)) => {
                here.remove(&self.surface);
                self.pane.replace(None);
                self.dialog.set_child(Some(&self.surface));
                if self.showing.get()
                    && let Some(window) = self.window.upgrade()
                {
                    self.dialog.present(Some(&window));
                }
            }
        }
        if let Some(window) = self.window.upgrade() {
            self.follow(&window);
        }
    }

    pub fn run(&self, id: CommandId) {
        for handler in self.commands.borrow().iter() {
            handler(id);
        }
    }

    pub fn showing(&self) -> bool {
        self.showing.get()
    }

    fn fit(dialog: &adw::Dialog, column: &adw::Clamp, in_pane: bool, width: i32, height: i32) {
        if width <= 0 || height <= 0 {
            return;
        }
        let wide = focus_dialog::dialog_width(width);
        if dialog.content_width() != wide {
            dialog.set_content_width(wide);
        }
        let tall = focus_dialog::dialog_height(height);
        if dialog.content_height() != tall {
            dialog.set_content_height(tall);
        }
        let host = if in_pane {
            focus_dialog::pane_width(width).unwrap_or(wide)
        } else {
            wide
        };
        let measure =
            focus_dialog::column_width(host, postio_body::treatment::Treatment::AppColours);
        if column.maximum_size() != measure {
            column.set_maximum_size(measure);
            column.set_tightening_threshold(measure);
        }
    }

    fn follow(&self, window: &FocusWindow) {
        let (width, height) = match (window.width(), window.height()) {
            (width, height) if width > 0 && height > 0 => (width, height),
            _ => window.default_size(),
        };
        Self::fit(&self.dialog, &self.column, self.in_pane(), width, height);
        if self.following.get() {
            return;
        }
        let Some(surface) = window.surface() else {
            return;
        };
        self.following.set(true);
        let dialog = self.dialog.downgrade();
        let column = self.column.downgrade();
        let pane = Rc::clone(&self.pane);
        let window = window.downgrade();
        surface.connect_layout(move |_, _, _| {
            if let (Some(dialog), Some(column), Some(window)) =
                (dialog.upgrade(), column.upgrade(), window.upgrade())
            {
                let in_pane = pane.borrow().is_some();
                Self::fit(&dialog, &column, in_pane, window.width(), window.height());
            }
        });
    }
}

impl ComposerHost for DialogHost {
    fn parent(&self) -> Option<gtk::Window> {
        self.window.upgrade().map(|window| window.upcast())
    }

    fn install(&self, composer: &Composer) {
        composer.set_vexpand(true);
        // Hidden until it takes the pane: the composer is open exactly while it is visible.
        composer.set_visible(false);
        self.slot.append(composer);
    }

    // The detached window takes the whole surface (header, action row, composer).
    fn restore(&self, _composer: &Composer) {
        self.parked.set(false);
        let here = self.pane.borrow().clone();
        match here {
            Some(slot) => slot.append(&self.surface),
            None => self.dialog.set_child(Some(&self.surface)),
        }
    }

    fn remove(&self, _composer: &Composer) {
        self.parked.set(true);
        let here = self.pane.borrow().clone();
        match here {
            Some(slot) => slot.remove(&self.surface),
            None => self.dialog.set_child(None::<&gtk::Widget>),
        }
    }

    fn frame(&self) -> Option<gtk::Widget> {
        Some(self.surface.clone())
    }

    fn take_pane(&self) {
        if self.showing.replace(true) {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            if let Some(reading) = window.reading() {
                reading.set_covered(true);
            }
            self.follow(&window);
            if self.in_pane() {
                window.show_pane_page();
            } else {
                self.dialog.present(Some(&window));
            }
        }
        crate::a11y::teach_shortcuts(&self.surface);
        crate::motion::keep_to_budget(&self.surface);
    }

    fn release_pane(&self) {
        if !self.showing.replace(false) {
            return;
        }
        if !self.in_pane() {
            self.dialog.force_close();
        }
        // A field of a closed dialog would hold the keyboard, and the resolver's "typing wins"
        // would swallow the next key.
        if let Some(window) = self.window.upgrade() {
            if let Some(reading) = window.reading() {
                reading.set_covered(false);
            }
            window.show_pane_page();
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            match window.visible_dialog() {
                // The dialog's first control is a step button, which Space would press, walking
                // away from the message just answered.
                Some(under) => match window.reading().filter(|reading| reading.is_open()) {
                    Some(reading) if under.widget_name() == crate::open::DIALOG_NAME => {
                        reading.focus_message();
                    }
                    _ => {
                        under.grab_focus();
                    }
                },
                None => window.focus_cursor(),
            }
        }
    }

    fn command_for_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        window: &gtk::Window,
    ) -> Option<CommandId> {
        let main = self.window.upgrade()?;
        let typing = gtk::prelude::GtkWindowExt::focus(window)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
            || main.composer_body_has_keyboard();
        main.command_in(key, state, KeyContext::Composer, typing)
    }

    fn handle_key(
        &self,
        _key: gtk::gdk::Key,
        _state: gtk::gdk::ModifierType,
        _window: &gtk::Window,
    ) -> glib::Propagation {
        glib::Propagation::Proceed
    }

    fn add_action(&self, action: &gio::SimpleAction) {
        if let Some(window) = self.window.upgrade() {
            window.add_action(action);
        }
    }

    fn connect_command(&self, handler: Box<dyn Fn(CommandId)>) {
        self.commands.borrow_mut().push(handler);
    }

    fn keymap(&self) -> Keymap {
        self.window
            .upgrade()
            .map(|window| window.keymap())
            .unwrap_or_else(|| Keymap::defaults().clone())
    }
}
