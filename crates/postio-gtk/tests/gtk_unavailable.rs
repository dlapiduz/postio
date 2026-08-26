//! The screen a store that will not open puts in front of the user (#404).
//!
//! Its own file with one test function, for the reason `gtk_onboarding.rs`
//! gives: GTK is single-threaded and initialised once, so two `#[test]`s here
//! would race `adw::init()` and the loser would take the no-display branch
//! and pass without asserting anything.
//!
//! What the screen *says* needs no display and is asserted in the crate's own
//! unit tests; what needs one is that the button is wired, that a retry in
//! flight cannot be asked for twice, and that the sentence it was handed is
//! the sentence it shows.

use std::cell::Cell;
use std::rc::Rc;

use postio_gtk::unavailable::Unavailable;

#[test]
fn the_screen_shows_what_it_was_told_and_asks_to_try_again_once() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }

    // `SecretError::Locked`'s own words, which carry the unlock hint. The
    // screen must show them verbatim rather than composing a sentence of its
    // own -- there would be two copies of that wording to keep in step, and
    // the one on screen would be the stale one.
    let said = "the login keyring is locked, so Postio cannot read the \
                password for ada@example.com. Unlock it and try again";

    let screen = Unavailable::new();
    screen.set_reason(said);
    // The error's own words, opened as a sentence: `SecretError` writes to be
    // embedded after "failed because", and on a screen that lowercase first
    // word reads as a fragment. Everything after it -- including the address
    // -- is untouched.
    assert_eq!(
        screen.reason(),
        "The login keyring is locked, so Postio cannot read the password for \
         ada@example.com. Unlock it and try again"
    );

    let asked = Rc::new(Cell::new(0));
    screen.connect_retry({
        let asked = Rc::clone(&asked);
        move || asked.set(asked.get() + 1)
    });

    screen.retry();
    assert_eq!(asked.get(), 1, "the button is wired to something");

    // A retry is a D-Bus round trip against a service that may be waiting for
    // the user to type a passphrase into a prompt of its own. Until it comes
    // back, asking again would queue work the app has nothing to do with.
    screen.set_busy(true);
    assert!(screen.is_busy());
    screen.retry();
    assert_eq!(
        asked.get(),
        1,
        "a retry in flight cannot be asked for again"
    );

    screen.set_busy(false);
    screen.retry();
    assert_eq!(asked.get(), 2, "and once it settles, it can");
}
