//! The settings panel on a real display: it shows the real file, `Escape`
//! closes it, typing is validated as you type, and an edit writes back to
//! disk and comes back around live through the same watcher `$EDITOR` uses.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_cheatsheet.rs`.
//!
//! Section navigation and the validity-line formatting are unit-tested in
//! `src/settings.rs` with no display. What needs one here is the widget
//! around them, and the seam with the rest of the running app: that the
//! panel's own write reaches `crate::config`'s watcher exactly the way a
//! hand save does — `gtk_live_config.rs` already proves that watcher rebinds
//! a running keymap, so reusing its `binding` helper here is the proof that
//! the panel is not a second, parallel path to the same file.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

/// How long to give the write debounce and the watcher's own debounce
/// together. Generous: this is a correctness check, not a latency budget.
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

pub fn the_settings_panel_edits_the_file_in_place() {
    let root = std::env::temp_dir().join(format!("postio-settings-{}", std::process::id()));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

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
    let original = "# a hand-written comment, above the key it explains\n[keys]\narchive = \"a\"\n\n[ui]\ndensity = \"compact\"\n";
    std::fs::write(&path, original).unwrap();

    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    settle();

    // ── loading shows the real file, byte for byte ─────────────────────────
    assert_eq!(
        window.settings().text(),
        original,
        "the panel and the file are the same thing"
    );
    assert!(window.settings().is_valid());
    assert!(!window.settings().is_visible(), "closed until asked for");

    // ── opening and Escape to close ─────────────────────────────────────────
    window.open_settings();
    settle();
    assert!(window.settings().is_visible());

    window.handle_key(gdk::Key::Escape, gdk::ModifierType::empty());
    settle();
    assert!(!window.settings().is_visible(), "Escape closes it");

    window.open_settings();
    settle();

    // ── typing is validated as you type, before any write settles ─────────
    window.settings().set_text("[keys\narchive = \"a\"\n");
    settle();
    assert!(
        !window.settings().is_valid(),
        "broken TOML shows invalid immediately"
    );

    // ── an edit writes back to disk, preserving the rest of the file ──────
    let edited = "# a hand-written comment, above the key it explains\n[keys]\narchive = \"y\"\n\n[ui]\ndensity = \"compact\"\n";
    window.settings().set_text(edited);
    settle();
    assert!(window.settings().is_valid());
    assert!(
        wait_until(|| std::fs::read_to_string(&path).unwrap_or_default() == edited),
        "the edit never reached disk: got {:?}",
        std::fs::read_to_string(&path)
    );

    // ── and it is applied live, through the same watcher `$EDITOR` uses ────
    assert!(
        wait_until(|| binding(&window, CommandId::Archive).as_deref() == Some("y")),
        "the panel's own write never came back around through the config watcher"
    );
    assert_eq!(
        binding(&window, CommandId::Archive).as_deref(),
        Some("y"),
        "not still the value from before the edit"
    );

    // ── Ctrl+E launches $EDITOR on the real path ───────────────────────────
    // A fake editor that records the one argument it was called with, rather
    // than an interactive one: this proves the command actually reaches a
    // process with the config path, without needing a real terminal editor
    // installed in whatever environment runs the test.
    let marker = root.join("editor-invoked");
    let fake_editor = root.join("fake-editor.sh");
    std::fs::write(
        &fake_editor,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > {:?}\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // SAFETY: single-threaded test; nothing else reads $EDITOR here.
    unsafe { std::env::set_var("EDITOR", &fake_editor) };

    window.handle_key(gdk::Key::e, gdk::ModifierType::CONTROL_MASK);
    assert!(
        wait_until(|| marker.is_file()),
        "Ctrl+E never launched $EDITOR"
    );
    assert!(
        wait_until(
            || std::fs::read_to_string(&marker).unwrap_or_default() == path.display().to_string()
        ),
        "the editor was not launched on the real config path: got {:?}",
        std::fs::read_to_string(&marker)
    );

    // ── Revert restores the last configuration that loaded without error ──
    window.settings().set_text("[keys\nbroken");
    settle();
    assert!(
        !window.settings().is_valid(),
        "the broken edit is on screen"
    );

    window.settings().revert();
    settle();
    assert!(
        window.settings().is_valid(),
        "revert did not restore validity"
    );
    assert_eq!(
        window.settings().text(),
        edited,
        "revert did not restore the last-good text"
    );
    assert!(
        wait_until(|| std::fs::read_to_string(&path).unwrap_or_default() == edited),
        "revert never reached disk"
    );
    assert!(
        window
            .settings()
            .footer_text()
            .to_lowercase()
            .contains("reverted"),
        "revert did not say so: {:?}",
        window.settings().footer_text()
    );

    // ── and it restores a save that came from outside the panel, too ──────
    // Simulates `$EDITOR` saving the file directly, the same shape
    // `write_atomically` uses: a temp file renamed over the target, which is
    // what the watcher is built to notice.
    let from_editor = "[keys]\narchive = \"a\"\n\n[ui]\ndensity = \"comfortable\"\n";
    let tmp = config_dir.join(".config.toml.tmp");
    std::fs::write(&tmp, from_editor).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    assert!(
        wait_until(|| binding(&window, CommandId::Archive).as_deref() == Some("a")),
        "the external save never reached the running app"
    );

    window.settings().set_text("[keys\nstill broken");
    settle();
    assert!(!window.settings().is_valid());

    window.settings().revert();
    settle();
    assert_eq!(
        window.settings().text(),
        from_editor,
        "revert did not know about a save `$EDITOR` made outside the panel"
    );

    window.close();
    settle();
    let _ = std::fs::remove_dir_all(&root);
}

pub fn a_keymap_problem_shows_up_on_the_settings_footer_not_only_a_debug_log() {
    // A binding the resolver dropped is not a debug log line nobody reads
    // interactively -- it is a setting the user wrote that did not take
    // effect, and the settings panel is where they would go to fix it.
    let root = std::env::temp_dir().join(format!("postio-settings-keymap-{}", std::process::id()));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

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
    // A misspelled command name. `postio_config::validate` cannot catch
    // this: `postio-config` sits below `postio-core` in the crate graph
    // (CLAUDE.md's architectural invariants) and has no way to know which
    // command ids exist, only its own static mirror of default *bindings*
    // for the ones it does. `postio_core::Keymap::resolve` has the real
    // registry and reports exactly this in `problems()` -- "not a command
    // in this build" -- which is the report() this issue is about.
    std::fs::write(&path, "[keys]\narchiv = \"z\"\n").unwrap();

    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    settle();

    assert!(
        window.settings().is_valid(),
        "the TOML itself is well-formed; `postio_config::validate` has no \
         business flagging a command name it cannot check"
    );

    let footer = window.settings().footer_text();
    assert!(
        footer.to_lowercase().contains("keymap"),
        "a dropped binding never reached the settings panel: {footer:?}"
    );
    assert!(
        footer.contains("archiv") && footer.contains("not a command"),
        "the footer does not name the unrecognised command: {footer:?}"
    );

    window.close();
    settle();
    let _ = std::fs::remove_dir_all(&root);
}
