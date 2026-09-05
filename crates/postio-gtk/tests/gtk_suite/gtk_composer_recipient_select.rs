//! #424: a suggestion can actually be *taken*, and is not offered too eagerly.
//!
//! `gtk_composer_recipients.rs` already covers that typing offers suggestions
//! and that accepting one edits only the token being typed. What was missing
//! is everything about choosing one:
//!
//!   * clicking a row did nothing at all — `populate` built plain `Label`s and
//!     nothing was ever connected to `row-activated`, so the mouse could move
//!     the selection and never commit it;
//!   * the popover opened on the first character typed, which means a database
//!     query per keystroke and a popover covering the field while there is not
//!     yet enough typed for the answer to mean anything.
//!
//! On Return: what is asserted here is that the *handler* commits the selected
//! suggestion. Whether a real Return reaches that handler is a question about
//! the key controller's propagation phase, and GTK4 offers a test no way to
//! synthesize a keystroke for a particular widget — see
//! `Composer::test_press_recipient_key`, and the note in `composer.rs` where
//! the phase is set.
//!
//! Skips without a display. Nothing here touches the network.

use crate::settle;
use std::cell::Cell;
use std::rc::Rc;

use gtk::gdk;
use postio_gtk::composer::{self, Composer, RecipientCandidate};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;

fn grace() -> EmailAddress {
    EmailAddress::new(Some("Grace Hopper"), "grace@example.com")
}

fn graham() -> EmailAddress {
    EmailAddress::new(Some("Graham Bell"), "graham@example.net")
}

/// A composer with two suggestions on offer, and a count of how many times the
/// suggestions provider was actually consulted.
fn a_composer_offering_two() -> Option<(Window, Composer, Rc<Cell<usize>>)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    settle();

    let asked = Rc::new(Cell::new(0usize));
    composer.connect_recipient_suggestions({
        let asked = Rc::clone(&asked);
        move |prefix| {
            asked.set(asked.get() + 1);
            // Answers for every prefix of "graham", so that anything not
            // offered is the gate's doing and never the provider's.
            if "graham".starts_with(prefix) {
                vec![
                    RecipientCandidate::Contact(grace()),
                    RecipientCandidate::Contact(graham()),
                ]
            } else {
                Vec::new()
            }
        }
    });

    Some((window, composer, asked))
}

/// Clicking a suggestion puts *that* suggestion in the field.
///
/// Row 1 rather than row 0 on purpose: row 0 is what `populate` selects by
/// default, so committing row 0 would also be what a handler that ignored the
/// click entirely would produce.
pub fn clicking_a_suggestion_puts_that_one_in_the_field() {
    let Some((_window, composer, _asked)) = a_composer_offering_two() else {
        return;
    };

    composer.test_set_to("grah");
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "four characters should have offered Grace and Graham"
    );
    assert_eq!(composer.test_recipient_suggestion_count(), 2);

    assert!(
        composer.test_click_recipient_suggestion(1),
        "there should be a second row to click"
    );
    settle();

    assert_eq!(
        composer.draft().to,
        vec![graham()],
        "clicking the second suggestion should commit the second suggestion"
    );
    assert!(
        !composer.test_recipient_popover_visible(),
        "committing a suggestion closes the popover"
    );
}

/// Return commits whatever the popover has selected, not merely the default.
pub fn return_commits_the_suggestion_the_popover_has_selected() {
    let Some((_window, composer, _asked)) = a_composer_offering_two() else {
        return;
    };

    composer.test_set_to("grah");
    settle();
    assert!(composer.test_recipient_popover_visible());

    // Down first, so this cannot pass by accepting the default row.
    assert!(
        composer.test_press_recipient_key(gdk::Key::Down),
        "Down belongs to the popover while it is open"
    );
    assert!(
        composer.test_press_recipient_key(gdk::Key::Return),
        "Return belongs to the popover while it is open"
    );
    settle();

    assert_eq!(
        composer.draft().to,
        vec![graham()],
        "Return should commit the row Down moved to"
    );
    assert!(!composer.test_recipient_popover_visible());
}

/// A composer offering one group, "Family", with two members.
fn a_composer_offering_a_group() -> Option<(Window, Composer)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    settle();

    composer.connect_recipient_suggestions(move |prefix| {
        if "family".starts_with(prefix) {
            vec![RecipientCandidate::Group {
                name: "Family".to_string(),
                members: vec![grace(), graham()],
            }]
        } else {
            Vec::new()
        }
    });

    Some((window, composer))
}

/// Picking a group inserts every member as its own address, not a group
/// reference (ADR 0007 Q3) — the whole point of expanding at pick time.
pub fn accepting_a_group_inserts_every_member() {
    let Some((_window, composer)) = a_composer_offering_a_group() else {
        return;
    };

    composer.test_set_to("fami");
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "four characters should have offered the group"
    );

    assert!(composer.test_click_recipient_suggestion(0));
    settle();

    assert_eq!(
        composer.draft().to,
        vec![grace(), graham()],
        "accepting the group inserts both members, not a group reference"
    );
    assert!(!composer.test_recipient_popover_visible());
}

/// Nothing is offered — and nothing is even looked up — before four
/// characters of the current token are typed.
pub fn nothing_is_offered_until_four_characters_are_typed() {
    let Some((_window, composer, asked)) = a_composer_offering_two() else {
        return;
    };

    for prefix in ["g", "gr", "gra"] {
        composer.test_set_to(prefix);
        settle();
        assert!(
            !composer.test_recipient_popover_visible(),
            "{prefix:?} is shorter than four characters and must offer nothing"
        );
    }
    assert_eq!(
        asked.get(),
        0,
        "a short token must not reach the suggestions provider at all -- that \
         is a database query on every keystroke"
    );

    composer.test_set_to("grah");
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "the fourth character is where suggestions start"
    );
    assert!(asked.get() > 0, "and the provider is consulted then");

    // The gate is on the token being typed, not on the whole field: a second
    // recipient starts counting from scratch after the comma.
    let existing = postio_model::address::format_list(&composer.draft().to);
    composer.test_set_to(&format!("{existing}, gr"));
    settle();
    assert!(
        !composer.test_recipient_popover_visible(),
        "a fresh two-character token is still too short, however long the \
         field already is"
    );
}
