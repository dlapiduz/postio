//! `[keys]` applied live, all the way from a file on disk to a key press.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_cheatsheet.rs`.
//!
//! This is the end of the chain the other tests each cover a link of. A real
//! `config.toml` is written to a real directory, a real `notify` watcher sees
//! it, the reparse happens on the watcher's own thread, and the result crosses
//! to the main context and changes what a key press does. Nothing is mocked,
//! because what is being tested is precisely that the pieces are joined up.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_config::Density;
use postio_core::CommandId;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

/// How long to give the watcher. Its debounce is 120ms and `notify` adds the
/// kernel's own latency; this is a ceiling, not an expectation — the loop below
/// leaves as soon as the change lands.
const PATIENCE: Duration = Duration::from_secs(5);

/// Runs the main loop until `condition` holds, or gives up.
fn wait_until(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// Pumps the main loop for `grace`, for an assertion that nothing happens.
///
/// A negative assertion has nothing to wait *for*, so it cannot leave early;
/// this is deliberately much shorter than [`PATIENCE`] so one of them does not
/// dominate the suite. Comfortably longer than the watcher's 120ms debounce.
fn settle_for(grace: Duration) {
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        settle();
        std::thread::sleep(Duration::from_millis(10));
    }
    settle();
}

/// The binding the window currently has for a command, as the cheat sheet
/// would print it — which is the live keymap, not the registry default.
fn binding(window: &Window, id: CommandId) -> Option<String> {
    window
        .cheatsheet()
        .sections()
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.id == Some(id.into()))
        .and_then(|row| row.binding)
}

pub fn editing_config_toml_rebinds_the_running_window() {
    let root = std::env::temp_dir().join(format!("postio-live-{}", std::process::id()));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };
    // `install_at` now wires `Ctrl+E` to actually launch `$EDITOR`
    // (postio-skc); this test presses it, and must not inherit whatever a
    // developer's own shell happens to have `$EDITOR` set to. `true` runs
    // and exits instantly, which is all a test that only checks the command
    // was dispatched needs.
    // SAFETY: same statement group as above.
    unsafe {
        std::env::set_var("EDITOR", "true");
        std::env::remove_var("VISUAL");
    }

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let path = config_dir.join("config.toml");
    std::fs::write(&path, "[keys]\narchive = \"y\"\n").unwrap();

    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    settle();

    // ── what was on disk at startup is in force ───────────────────────────
    assert_eq!(
        binding(&window, CommandId::Archive).as_deref(),
        Some("y"),
        "the file's binding, not the registry default"
    );

    let ran: std::rc::Rc<std::cell::RefCell<Vec<CommandId>>> = Default::default();
    window.connect_command({
        let ran = std::rc::Rc::clone(&ran);
        move |id| ran.borrow_mut().push(id)
    });

    window.handle_key(
        gdk::Key::from_name("y").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(
        *ran.borrow(),
        vec![CommandId::Archive],
        "and it is pressable"
    );

    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(
        ran.borrow().len(),
        1,
        "the default it replaced is gone, not still lying around"
    );

    // ── the user edits the file ───────────────────────────────────────────
    std::fs::write(&path, "[keys]\narchive = \"w\"\n").unwrap();
    assert!(
        wait_until(|| binding(&window, CommandId::Archive).as_deref() == Some("w")),
        "the edit never reached the window: got {:?}",
        binding(&window, CommandId::Archive)
    );

    ran.borrow_mut().clear();
    window.handle_key(
        gdk::Key::from_name("w").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(
        *ran.borrow(),
        vec![CommandId::Archive],
        "the new key works with no restart"
    );

    window.handle_key(
        gdk::Key::from_name("y").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(ran.borrow().len(), 1, "and the old one has stopped working");

    // ── a broken file leaves the working keys alone ───────────────────────
    std::fs::write(&path, "[keys\narchive = ").unwrap();
    // Nothing to wait *for* — the assertion is that nothing changes — so give
    // the watcher a moment and then check it did not take the keyboard away.
    settle_for(Duration::from_millis(600));

    assert_eq!(
        binding(&window, CommandId::Archive).as_deref(),
        Some("w"),
        "a half-saved file must leave the last good keymap in force"
    );
    ran.borrow_mut().clear();
    window.handle_key(
        gdk::Key::from_name("e").unwrap(),
        gdk::ModifierType::CONTROL_MASK,
    );
    settle();
    assert_eq!(
        *ran.borrow(),
        vec![CommandId::EditConfig],
        "including Ctrl+E, which is what the user needs to fix it"
    );

    // ── and it recovers when the save finishes ────────────────────────────
    std::fs::write(&path, "[keys]\narchive = \"q\"\n").unwrap();
    assert!(
        wait_until(|| binding(&window, CommandId::Archive).as_deref() == Some("q")),
        "the window never recovered from the broken save"
    );

    // ── `[ui].density` is live too, all the way to the list ───────────────
    assert_eq!(
        window.list().density(),
        Density::Airy,
        "nothing on disk yet, so the PLATE default is in force"
    );

    std::fs::write(
        &path,
        "[keys]\narchive = \"q\"\n\n[ui]\ndensity = \"compact\"\n",
    )
    .unwrap();
    assert!(
        wait_until(|| window.list().density() == Density::Compact),
        "the density edit never reached the list"
    );

    std::fs::write(
        &path,
        "[keys]\narchive = \"q\"\n\n[ui]\ndensity = \"comfortable\"\n",
    )
    .unwrap();
    assert!(
        wait_until(|| window.list().density() == Density::Comfortable),
        "switching density again never reached the list"
    );

    // ── `[keys]` reaches the list's own keymap too, not just the resolver ──
    assert_eq!(
        window.list().keymap().binding(CommandId::Archive),
        Some("q"),
        "the list should already carry the rebind from earlier in this test"
    );
    std::fs::write(
        &path,
        "[keys]\narchive = \"z\"\n\n[ui]\ndensity = \"comfortable\"\n",
    )
    .unwrap();
    assert!(
        wait_until(|| window.list().keymap().binding(CommandId::Archive) == Some("z")),
        "the rebind never reached the list's keymap, so a row's hint would lie"
    );

    window.close();
    settle();
    let _ = std::fs::remove_dir_all(&root);
}
