//! Debounced autosave: typing coalesces into one save, and closing never
//! leaves an edit sitting only in a timer.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.
//!
//! What this proves, within what a composer with no storage of its own can
//! prove: [`postio_gtk::composer::Composer::connect_save`] is the same
//! seam `ctrl+s` uses, so whatever wires it to `DraftRepository` gets both
//! the explicit save and the autosave for free. Actually writing to the
//! drafts table, recovering on restart, and syncing to the server Drafts
//! folder are `postio-core`'s side of postio-own; this is the composer's.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// Pumps the main loop, including its timers, until `condition` holds or
/// `timeout` passes. What a test does instead of a fixed real-time sleep when
/// it is waiting on a glib timer rather than on a fixed clock — it returns as
/// soon as the debounce actually fires rather than however long `timeout` is.
fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        glib::MainContext::default().iteration(true);
    }
}

#[test]
fn typing_debounces_into_one_autosave_and_closing_flushes_what_is_pending() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-composer-autosave-{}", std::process::id()));
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
    window.present();
    settle();

    let composer = composer::install(&window);
    let saves: std::rc::Rc<std::cell::RefCell<Vec<Draft>>> = Default::default();
    composer.connect_save({
        let saves = std::rc::Rc::clone(&saves);
        move |draft| saves.borrow_mut().push(draft.clone())
    });

    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();
    assert!(composer.is_open());

    // ── A burst of edits arms exactly one pending autosave ───────────────
    for text in ["h", "he", "hel", "hell", "hello"] {
        composer.test_set_subject(text);
        settle();
    }
    assert!(
        saves.borrow().is_empty(),
        "autosave must not fire on every keystroke — that is what blocking would look like"
    );

    // ── …and it does fire, on its own, once typing stops ─────────────────
    let fired = wait_until(Duration::from_secs(5), || !saves.borrow().is_empty());
    assert!(fired, "the debounced autosave never ran");
    assert_eq!(saves.borrow().last().unwrap().subject, "hello");
    saves.borrow_mut().clear();

    // ── Closing before the debounce elapses flushes it immediately ───────
    composer.test_set_subject("hello!");
    settle();
    assert!(
        saves.borrow().is_empty(),
        "the second edit's debounce has not elapsed yet"
    );
    // `Esc` closes the composer and keeps the draft; if the app were killed
    // right now, the flush below is what stands between "hello!" and nothing.
    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert!(
        !saves.borrow().is_empty(),
        "closing must flush a pending autosave rather than leave it in a timer a crash could lose"
    );
    assert_eq!(saves.borrow().last().unwrap().subject, "hello!");
}
