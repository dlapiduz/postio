//! The composer the classic window holds (specs/007-postio-focus T024).
//!
//! The composer itself is postio-widgets' (ADR 0043), and runs on any
//! [`ComposerHost`]. What is here is the classic window as one:
//! [`WindowHost`] takes the window's reading pane, its keyboard context and
//! its keymap, and [`install`] mounts a composer on it. Everything else is
//! re-exported, so every path that named `postio_gtk::composer` still does.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_core::{CommandId, Context, Keymap};

pub use postio_widgets::composer::*;

use crate::shell::Pane;
use crate::window::Window;

/// The classic window as a composer's host: its reading pane, its keyboard
/// context and its keymap.
struct WindowHost {
    window: glib::WeakRef<Window>,
    /// The context and pane to put back when the composer closes.
    restore: Cell<Option<(Context, Pane)>>,
}

impl ComposerHost for WindowHost {
    fn parent(&self) -> Option<gtk::Window> {
        self.window.upgrade().map(|window| window.upcast())
    }

    fn install(&self, composer: &Composer) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let reader = window.shell().reader();
        composer.set_vexpand(true);
        reader.append(composer);
        // The pane's arbiter owns this widget's visibility from here on
        // (#502): hidden until the composer claims the pane.
        window.shell().register_reader_occupant(
            crate::shell::ReaderOccupant::Composer,
            composer.upcast_ref(),
        );
    }

    fn restore(&self, composer: &Composer) {
        if let Some(window) = self.window.upgrade() {
            window.shell().reader().append(composer);
        }
    }

    fn remove(&self, composer: &Composer) {
        if let Some(window) = self.window.upgrade() {
            window.shell().reader().remove(composer);
        }
    }

    // Hides whatever else is in the reading pane and remembers the way back.
    fn take_pane(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let shell = window.shell();
        self.restore
            .set(Some((window.context(), shell.focused_pane())));

        // Through the pane's one owner (#502): the shell hides whichever
        // occupant had the pane and shows this composer. The old shape —
        // hide every sibling here, show every sibling on release — is what
        // put a search preview back under an open message.
        shell.set_composing(true);
        // In the one-pane mode the reader is not necessarily on screen, and a
        // composer the user cannot see is the worst possible mode.
        shell.set_focused_pane(Pane::Reader);
        shell.add_css_class(COMPOSING_CLASS);
        window.set_context(Context::Composer);
    }

    // Gives the reading pane back to whatever is active now.
    fn release_pane(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        // Computed, not replayed: the shell shows what the current state
        // calls for — the search preview if search is up, the message the
        // pane was open on, or nothing. Showing every sibling here is the
        // #502 bug.
        window.shell().set_composing(false);
        window.shell().remove_css_class(COMPOSING_CLASS);
        if let Some((context, pane)) = self.restore.take() {
            window.set_context(context);
            window.shell().set_focused_pane(pane);
        }

        // The keyboard is still in one of the composer's fields, which is
        // about to be a *hidden* text entry — and the resolver's "typing
        // always wins" rule would then swallow the next single-key binding as
        // a character typed into something nobody can see. Dropping the focus
        // first is what makes `c` after `Esc` open the composer again rather
        // than type a `c` into it.
        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        window.shell().grab_focus();
    }

    fn command_for_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        window: &gtk::Window,
    ) -> Option<CommandId> {
        self.window
            .upgrade()?
            .command_for_key_in(key, state, window, Context::Composer)
    }

    fn handle_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        window: &gtk::Window,
    ) -> glib::Propagation {
        match self.window.upgrade() {
            Some(main) => main.handle_key_in(key, state, window, Context::Composer),
            None => glib::Propagation::Proceed,
        }
    }

    fn add_action(&self, action: &gio::SimpleAction) {
        if let Some(window) = self.window.upgrade() {
            window.add_action(action);
        }
    }

    fn connect_command(&self, handler: Box<dyn Fn(CommandId)>) {
        if let Some(window) = self.window.upgrade() {
            window.connect_command(handler);
        }
    }

    fn keymap(&self) -> Keymap {
        self.window
            .upgrade()
            .map(|window| window.keymap_in_force())
            .unwrap_or_else(|| Keymap::defaults().clone())
    }

    fn composing(&self, open: bool, keymap: &Keymap) {
        if let Some(button) = self.window.upgrade().and_then(|w| w.compose_button()) {
            crate::header::sync_compose(&button, open, keymap);
        }
    }

    // The classic stylesheet keys its schemes off classes on a window's
    // root, and only a tracked window carries them: the settings window is
    // tracked the same way.
    fn adopt(&self, window: &gtk::Window) {
        crate::style::track(window);
    }
}

/// Puts `composer` in `window`'s reading pane and wires the keyboard.
///
/// After this, `c` opens the composer, `Esc` closes it keeping the draft,
/// `ctrl+Enter` sends, `ctrl+s` saves and `ctrl+d` asks before discarding —
/// all through the command registry, so the palette and the cheat sheet
/// say the same thing the keys do. The header's Compose button reaches the
/// same place through the `win.compose` action.
pub fn mount(composer: &Composer, window: &Window) {
    composer.mount_on(Rc::new(WindowHost {
        window: window.downgrade(),
        restore: Cell::new(None),
    }));
}

/// Installs a composer in `window` and returns it.
///
/// One call, because there is nothing to choose: the composer belongs in the
/// reading pane of the window that owns the keyboard.
pub fn install(window: &Window) -> Composer {
    let composer = Composer::new();
    mount(&composer, window);
    composer
}
