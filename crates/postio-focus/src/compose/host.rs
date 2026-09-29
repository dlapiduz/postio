//! Focus's compose dialog as a composer's host (research R15): the second
//! implementation of `ComposerHost`, beside the classic window's.
//!
//! Where the classic window lends the composer its reading pane, Focus lends
//! it a dialog over the list: taking the pane presents the dialog, and
//! giving it back closes it and returns the keyboard to the row it left
//! from. The keymap is the window's, resolved in the composer's context, so
//! `[keys]` reaches the composer here exactly as it does there.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use gtk::{gio, glib};
use postio_core::{CommandId, Keymap};
use postio_ui::keymap::KeyContext;
use postio_widgets::composer::{Composer, ComposerHost};

use crate::window::FocusWindow;

/// Who hears a command the window hands the composer.
type CommandHandler = Box<dyn Fn(CommandId)>;

/// The dialog, the slot in it the composer sits in, and the window under it.
pub struct DialogHost {
    pub(super) window: glib::WeakRef<FocusWindow>,
    pub(super) dialog: adw::Dialog,
    pub(super) slot: gtk::Box,
    /// Who hears the commands the window dispatches to the composer.
    commands: RefCell<Vec<CommandHandler>>,
    /// Whether the dialog is over the window: between taking the pane and
    /// giving it back.
    showing: Cell<bool>,
}

impl DialogHost {
    pub fn new(window: &FocusWindow, dialog: adw::Dialog, slot: gtk::Box) -> Self {
        DialogHost {
            window: window.downgrade(),
            dialog,
            slot,
            commands: RefCell::default(),
            showing: Cell::new(false),
        }
    }

    /// Hand `id` to the composer, as a key or a control asked.
    pub fn run(&self, id: CommandId) {
        for handler in self.commands.borrow().iter() {
            handler(id);
        }
    }

    /// Whether the dialog is over the window.
    pub fn showing(&self) -> bool {
        self.showing.get()
    }
}

impl ComposerHost for DialogHost {
    fn parent(&self) -> Option<gtk::Window> {
        self.window.upgrade().map(|window| window.upcast())
    }

    fn install(&self, composer: &Composer) {
        composer.set_vexpand(true);
        // Hidden until it takes the pane: the composer is open exactly
        // while it is visible.
        composer.set_visible(false);
        self.slot.append(composer);
    }

    fn restore(&self, composer: &Composer) {
        self.slot.append(composer);
    }

    fn remove(&self, composer: &Composer) {
        self.slot.remove(composer);
    }

    fn take_pane(&self) {
        if self.showing.replace(true) {
            return;
        }
        if let Some(window) = self.window.upgrade() {
            self.dialog.present(Some(&window));
        }
        crate::a11y::teach_shortcuts(&self.dialog);
    }

    fn release_pane(&self) {
        if !self.showing.replace(false) {
            return;
        }
        self.dialog.force_close();
        // The keyboard goes back to the list, on the row it left from: a
        // field of a closed dialog would otherwise hold it, and the
        // resolver's "typing wins" would swallow the next key.
        if let Some(window) = self.window.upgrade() {
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            // Back to the message it was written from, when one is open;
            // to the list otherwise.
            match window.visible_dialog() {
                Some(under) => {
                    under.grab_focus();
                }
                None => {
                    if let Some(pane) = window.pane() {
                        pane.view().grab_focus();
                    }
                }
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

    // Focus's Compose button says nothing about whether a composition is
    // open: the dialog over the window says it.
    fn composing(&self, _open: bool, _keymap: &Keymap) {}

    // Focus's colours follow the system through libadwaita, for every
    // window of the process alike: nothing to join.
    fn adopt(&self, _window: &gtk::Window) {}
}
