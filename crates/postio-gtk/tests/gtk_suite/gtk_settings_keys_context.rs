//! The keybinding list answers the keyboard the same way the account list
//! already does (#881, #1016) -- `Context::Keys`, scoped to `keys_list`
//! rather than the whole settings panel for the exact reason
//! `gtk_settings_accounts_keys.rs` already gives `Context::Accounts`: the
//! panel also holds a `GtkTextView` of the literal `config.toml`, where a
//! bare-letter binding must insert a letter, not fire a command.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::window::Window;

use crate::pump;

/// A window with its settings panel open on a populated `[keys]` pane.
fn ready() -> Option<Window> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let window = Window::default();
    window.present();
    window.open_settings();
    // `load()`/`set_text()` is what actually populates `keys_list`'s rows
    // -- nothing in `Window::default()` calls it, the same reason
    // `gtk_settings_keys.rs`'s own tests seed the buffer themselves.
    window.settings().set_text("");
    pump();
    Some(window)
}

pub fn focus_on_a_keys_row_enters_the_keys_context_and_leaving_restores_it() {
    let Some(window) = ready() else { return };

    window.set_context(Context::List);
    let list = window.settings().keys_list();
    let row = list.row_at_index(0).expect("a first keybinding row");
    row.grab_focus();
    pump();

    assert_eq!(
        window.context(),
        Context::Keys,
        "the context must follow the keyboard into the keybinding list, or \
         a bare letter fires a command while the focus ring sits on a row"
    );

    window.list().grab_focus();
    pump();
    assert_eq!(
        window.context(),
        Context::List,
        "leaving the keybinding list must restore the context it \
         interrupted, not strand the window in Keys"
    );
}

pub fn a_bare_letter_binding_does_nothing_while_the_keyboard_is_on_a_keys_row_and_not_capturing() {
    let Some(window) = ready() else { return };

    let list = window.settings().keys_list();
    let row = list.row_at_index(0).expect("a first keybinding row");
    row.grab_focus();
    pump();
    assert_eq!(
        window.context(),
        Context::Keys,
        "setup: focus is on the row"
    );

    // "a" archives in Context::List, and Context::Keys has no command of
    // its own bound to it (#881 registers none) -- so once the context has
    // genuinely followed the focus, "a" must resolve to nothing at all
    // (`Outcome::Unhandled`, which the resolver reports as `Proceed`, not
    // `Stop`: a command that *did* resolve -- the leak this test exists to
    // catch -- returns `Stop` instead, from `Window::run_action`).
    let propagation = window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();

    assert_eq!(
        propagation,
        gtk::glib::Propagation::Proceed,
        "a bare letter resolving to a command at all, while the keyboard is \
         on a keybinding row, is the leak Context::Keys exists to close"
    );
}
