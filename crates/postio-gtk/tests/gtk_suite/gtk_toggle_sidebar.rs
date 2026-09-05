//! Toggling the sidebar from the palette and from `Ctrl+B` (#756).
//!
//! `ToggleSidebar` was a registry command with no `Window::handled_here`
//! arm, so both routes resolved the command correctly and then did nothing
//! with it. The existing palette test only asserted the command reached
//! `Window::connect_command` -- which an orphaned command does too, since
//! nothing further down the chain was checked -- so this drives the real
//! palette and the real key, and asserts the thing a person would actually
//! see: the sidebar moves.
//!
//! One test function: GTK is single-threaded and initialised once per
//! process. Skips without a display. Nothing here touches the network.

use crate::pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::Mode;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

fn press(window: &Window, key: &str, state: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), state);
    pump();
}

/// Type `text` into the header's field, one insertion, as a user would.
fn type_into(window: &Window, text: &str) {
    let entry = field(window);
    let position = entry.text().len() as i32;
    entry.set_text(&format!("{}{text}", entry.text()));
    entry.set_position(position + text.len() as i32);
}

fn field(window: &Window) -> gtk::Text {
    fn find(widget: &gtk::Widget) -> Option<gtk::Text> {
        if let Some(text) = widget.downcast_ref::<gtk::Text>() {
            return Some(text.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(window.upcast_ref::<gtk::Widget>()).expect("the header's one box")
}

pub fn toggle_sidebar_moves_the_sidebar_from_the_palette_and_from_ctrl_b() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    let shell = window.shell();
    let starting = shell.sidebar_visible();

    // ── the palette route ─────────────────────────────────────────────
    window.open_finder(Mode::Command);
    pump();
    type_into(&window, "toggle sidebar");
    pump();
    window.finder().activate();
    pump();
    assert_eq!(
        shell.sidebar_visible(),
        !starting,
        "running Toggle sidebar from the palette did not move the sidebar"
    );
    assert!(
        !window.finder().is_open(),
        "running a command closes the palette"
    );

    // ── `Ctrl+B`, the keyboard route the registry advertises ───────────
    press(&window, "b", gdk::ModifierType::CONTROL_MASK);
    assert_eq!(
        shell.sidebar_visible(),
        starting,
        "Ctrl+B did not toggle the sidebar back"
    );

    press(&window, "b", gdk::ModifierType::CONTROL_MASK);
    assert_eq!(
        shell.sidebar_visible(),
        !starting,
        "a second Ctrl+B should toggle it again"
    );

    window.destroy();
}
