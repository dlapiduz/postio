//! The `?` cheat sheet on a real display.
//!
//! Its own file, not another function in `gtk_palette.rs`: two `#[test]`
//! functions in one integration test share a process and run on separate
//! threads, and GTK is single-threaded and initialised once. A separate file is
//! a separate binary, which is a separate process.
//!
//! What the sheet *contains* is unit-tested in `src/cheatsheet.rs` with no
//! display. What needs one is the overlay around it: that `?` opens it, that
//! `?` and `Esc` both close it, and that a rebind reaches it.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{ActionId, CommandId, Keymap};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

fn defaults() -> Keymap {
    Keymap::resolve(&postio_config::KeyBindings::default())
}

/// Runs the main loop until everything queued has been done.
fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn the_cheat_sheet_opens_and_reprints_on_a_rebind() {
    let state_dir = std::env::temp_dir().join(format!("postio-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.apply_keymap(defaults());
    window.present();
    settle();

    assert!(!window.cheatsheet().is_visible());

    // `?` opens it, through the resolver.
    assert_eq!(
        window.handle_key(
            gdk::Key::from_name("question").unwrap(),
            gdk::ModifierType::SHIFT_MASK
        ),
        glib::Propagation::Stop
    );
    settle();
    assert!(window.cheatsheet().is_visible(), "? opens the sheet");

    let sections = window.cheatsheet().sections();
    assert!(!sections.is_empty(), "and it has something in it");
    let listed: Vec<ActionId> = sections
        .iter()
        // `filter_map`: a row's command is optional now, because the box's
        // prefixes are on the sheet and are not commands. Dropping the `None`s
        // is what keeps the count below a count of *registry* entries.
        .flat_map(|section| section.rows.iter().filter_map(|row| row.id))
        .collect();
    assert_eq!(
        listed.len(),
        postio_core::registry::all().count(),
        "every command, once each"
    );

    // `?` again closes it — the key that opened it has to be able to close it.
    window.handle_key(
        gdk::Key::from_name("question").unwrap(),
        gdk::ModifierType::SHIFT_MASK,
    );
    settle();
    assert!(!window.cheatsheet().is_visible());

    // Esc closes it too.
    window.handle_key(
        gdk::Key::from_name("question").unwrap(),
        gdk::ModifierType::SHIFT_MASK,
    );
    settle();
    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert!(!window.cheatsheet().is_visible(), "Esc closes it");

    // A rebind reaches the sheet with no code edit.
    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_owned(), "y".to_owned());
    window.apply_keymap(Keymap::resolve(&overrides));
    settle();

    let archive = window
        .cheatsheet()
        .sections()
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.id == Some(ActionId::Builtin(CommandId::Archive)))
        .expect("archive");
    assert_eq!(archive.binding.as_deref(), Some("y"));

    // Opening the sheet puts the box away, and vice versa.
    window.open_finder(postio_gtk::finder::Mode::Command);
    settle();
    window.open_cheatsheet();
    settle();
    assert!(window.cheatsheet().is_visible());
    assert!(
        !window.finder().is_open(),
        "two overlays at once is one too many"
    );

    window.close_cheatsheet();
    window.close();
    settle();
}
