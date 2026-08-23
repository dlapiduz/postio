//! The `/` query bar on a real display.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_cheatsheet.rs`.
//!
//! The chip rule and the Backspace rule are unit-tested in `src/search.rs` with
//! no display. What needs one is the bar around them: that `/` opens it, that
//! Backspace really pops a chip out of a real entry, and that `Esc` puts the
//! view back the way it was.

use std::time::Instant;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::shell::Pane;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

/// The interaction budget from CLAUDE.md. Used as a ceiling here, not as a
/// benchmark — `cargo bench` owns the real numbers.
const INTERACTION_BUDGET: std::time::Duration = std::time::Duration::from_millis(16);

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn the_query_bar_opens_chips_and_restores() {
    let state_dir = std::env::temp_dir().join(format!("postio-search-{}", std::process::id()));
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

    assert!(
        !window.search().is_visible(),
        "not in the way until asked for"
    );
    window.shell().set_focused_pane(Pane::Reader);
    window.set_context(Context::Reader);

    // ── `/` opens it, with no perceptible delay ───────────────────────────
    let start = Instant::now();
    assert_eq!(
        window.handle_key(
            gdk::Key::from_name("slash").unwrap(),
            gdk::ModifierType::empty()
        ),
        glib::Propagation::Stop
    );
    settle();
    let opened = start.elapsed();

    assert!(window.search().is_visible(), "/ opens the bar");
    assert!(
        opened < INTERACTION_BUDGET * 4,
        "/ to typeable took {opened:?}"
    );
    assert_eq!(
        window.context(),
        Context::Search,
        "so Esc means close-search rather than back"
    );

    // ── operators become chips as they are typed ──────────────────────────
    let bar = window.search();
    bar.set_query("from:ada@example.com report");
    settle();

    let chips = bar.chips();
    assert_eq!(chips.len(), 1, "one operator, one chip");
    assert_eq!(chips[0].label, "from:ada@example.com");
    assert!(chips[0].complete);

    bar.set_query("from:ada@example.com is:flagged report");
    settle();
    assert_eq!(bar.chips().len(), 2, "free text alongside them stays plain");

    // ── Backspace pops a whole chip, not a character ──────────────────────
    bar.set_query("report is:flagged");
    settle();
    assert!(
        bar.press_backspace(),
        "the caret is at the right edge of a chip"
    );
    settle();
    assert_eq!(
        bar.query(),
        "report",
        "the whole chip went, not the `d` off the end of it"
    );
    assert!(bar.chips().is_empty());

    // …and in free text it does not.
    assert!(
        !bar.press_backspace(),
        "in `report` the entry deletes a character, as it always has"
    );
    assert_eq!(bar.query(), "report", "and the bar left the entry to it");

    // ── typing is not a command ───────────────────────────────────────────
    // `a` archives in the list. In the bar it is the letter A.
    let ran: std::rc::Rc<std::cell::RefCell<Vec<postio_core::CommandId>>> = Default::default();
    window.connect_command({
        let ran = std::rc::Rc::clone(&ran);
        move |id| ran.borrow_mut().push(id)
    });
    bar.grab_entry_focus();
    settle();
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert!(
        ran.borrow().is_empty(),
        "a single-key binding fired while the user was typing a query"
    );

    // ── Esc restores the view it opened over ──────────────────────────────
    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();

    assert!(!window.search().is_visible(), "Esc closes the bar");
    assert_eq!(
        window.context(),
        Context::Reader,
        "and puts the keyboard back where it was"
    );
    assert_eq!(window.shell().focused_pane(), Pane::Reader);

    // ── reopening starts clean ────────────────────────────────────────────
    window.open_search();
    settle();
    assert_eq!(
        window.search().query(),
        "",
        "a bar that reopens showing the last search has to be cleared first"
    );

    // ── the budget for typing ─────────────────────────────────────────────
    let start = Instant::now();
    for query in [
        "f",
        "fr",
        "fro",
        "from",
        "from:",
        "from:a",
        "from:ad",
        "from:ada@example.com",
        "from:ada@example.com is:",
        "from:ada@example.com is:flagged report",
    ] {
        window.search().set_query(query);
        settle();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < INTERACTION_BUDGET * 10,
        "ten reparses and redraws took {elapsed:?}"
    );

    window.close_search();
    window.close();
    settle();
}
