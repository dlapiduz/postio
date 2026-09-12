//! A real window with no store behind it (#1114).
//!
//! Postio presents its window before it has opened anything — the keyring
//! read, the schema migrations and the search-index rebuild all happen behind
//! a window that already exists, and on the live install a migration launch
//! spent 12.6 s with nothing on screen at all. What the window does in that
//! interval is decided here rather than being whatever falls out.
//!
//! What the *copy* says and when the plate is due are unit-tested in
//! `src/list_state.rs` with no display. What needs one is the window around
//! them: that an ordinary start draws nothing that is then removed, that the
//! plate arrives on its own timer, and that a key for a command that cannot
//! run says so rather than being swallowed.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Set as the first statement of a single-threaded case, which
// is the one moment it is sound.

use crate::settle;
use gtk::gdk;
use gtk::prelude::*;
use postio_core::Keymap;
use postio_gtk::list_state::{OPENING_THRESHOLD, State, Waiting, describe_wait};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

fn a_window() -> Option<Window> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.apply_keymap(Keymap::resolve(&postio_config::KeyBindings::default()));
    Some(window)
}

/// #1114's first acceptance line, and the one a screenshot cannot settle.
///
/// *An ordinary start draws nothing that is then removed.* The measured store
/// phase is tens of milliseconds; anything drawn and taken away inside that is
/// flicker, and `docs/PRODUCT.md` §18 allows a transition of ≤100 ms **or
/// none**. This path adds none.
///
/// The state it guards against is not hypothetical: a list pane with no sync
/// status yet derives [`State::Offline`] from its own defaults, so before
/// #1114 a window presented ahead of its store would have drawn a full-pane
/// "Offline — reading local mail" over mail it had not opened, and then
/// replaced it.
pub fn an_ordinary_start_draws_nothing_that_is_then_removed() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
    let Some(window) = a_window() else { return };

    window.set_waiting_on(Waiting::Keyring);
    window.present();
    settle();

    assert_eq!(
        window.list_state().state(),
        None,
        "the list pane said something on a window whose store has not opened \
         yet, and whatever it said will be replaced the moment one does"
    );
    assert!(
        !window.list_state().is_visible(),
        "a plate that is empty is still a plate: it covers the pane the rows \
         are about to arrive in"
    );
    // And no counts. #1114 asks for the sidebar's structure without them,
    // because a count that appears late must not be seen jumping from `0` —
    // which is a promise that holds by construction only for as long as
    // nothing decides an unfed sidebar should show a placeholder.
    assert!(
        window.sidebar().mailboxes().is_empty(),
        "the sidebar has folders to count before anything has read any"
    );

    // And the store lands, as it does on every ordinary start, well inside
    // the threshold. What the pane says from here on is the ordinary
    // business of the sync feed -- an unfed window still says "Offline", as
    // it always has -- but it must no longer be claiming to be opening
    // anything, and nothing must still be being waited for.
    window.set_store_open(true);
    settle();
    assert_eq!(window.list_state().waiting(), None);
    assert!(
        !matches!(window.list_state().state(), Some(State::Opening { .. })),
        "the plate outlived the wait it was about"
    );

    window.close();
    settle();
}

/// The plate arrives on its own timer, saying which wait it is.
///
/// Driven by winding the wait back rather than by sleeping for a second: a
/// test that waits out a real threshold is a test that either takes a second
/// or races a loaded runner into failing.
pub fn a_start_past_its_budget_says_what_it_is_waiting_on() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
    let Some(window) = a_window() else { return };

    window.set_waiting_on(Waiting::Migrating);
    window.present();
    settle();
    assert_eq!(window.list_state().state(), None, "not yet");

    window.list_state().wind_back(OPENING_THRESHOLD);
    settle();
    assert_eq!(
        window.list_state().state(),
        Some(State::Opening {
            waiting: Waiting::Migrating
        }),
        "past twice the startup budget with the store still opening, the \
         window owes the reader a sentence"
    );
    assert!(window.list_state().is_visible());

    window.close();
    settle();
}

/// #1114's keyboard rule: *refuse out loud rather than be swallowed.*
///
/// The precedent is `registry.rs`'s own note on reply in the composer (#426):
/// a key that resolves to a command that cannot run must get the chance to
/// say so, because a key that does nothing is indistinguishable from a key
/// that is not bound — which is how "it randomly stopped working" gets
/// reported.
pub fn a_key_for_mail_says_why_it_cannot_run_yet() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };
    let Some(window) = a_window() else { return };

    let ran: std::rc::Rc<std::cell::RefCell<Vec<postio_core::CommandId>>> = Default::default();
    window.connect_command({
        let ran = ran.clone();
        move |id| ran.borrow_mut().push(id)
    });

    window.set_waiting_on(Waiting::Keyring);
    window.present();
    settle();

    // `a` archives, and there is nothing to archive because there is nothing
    // open to have read it from.
    assert_eq!(
        window.handle_key(
            gdk::Key::from_name("a").unwrap(),
            gdk::ModifierType::empty()
        ),
        glib::Propagation::Stop,
        "the key is bound, so it is this window's to answer for"
    );
    settle();

    assert!(
        ran.borrow().is_empty(),
        "archive reached a handler with no store behind the window: {:?}",
        ran.borrow()
    );
    let showing = window
        .toast()
        .and_then(|toast| toast.showing())
        .expect("a key that cannot run has to say so, not go quiet");
    assert_eq!(
        showing.title().as_deref(),
        Some(describe_wait(Waiting::Keyring).1),
        "and it says the same sentence the plate would, so the two surfaces \
         cannot describe the same wait differently"
    );

    // Once the store is open the same key reaches its handler.
    window.set_store_open(true);
    settle();
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(
        ran.borrow().as_slice(),
        &[postio_core::CommandId::Archive],
        "the refusal is a wait, not a removal"
    );

    window.close();
    settle();
}
