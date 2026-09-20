//! Recipient completion: a prefix in `To` searches whatever
//! [`Composer::connect_recipient_suggestions`] is wired to, offers what it
//! returns, and accepting one replaces only the address being typed.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.
//!
//! Every prefix here is four characters or more: below
//! `MIN_COMPLETION_PREFIX` nothing is offered and the provider is not even
//! consulted (#424), so a shorter prefix would prove nothing about matching.
//! The threshold itself, and taking a suggestion by click or by Return, are
//! covered in `gtk_suite/gtk_composer_recipient_select.rs`.
//!
//! `current_entry`'s own splitting rules are unit-tested in
//! `postio-model`'s `address.rs` with no display; what needs one here is
//! that typing actually shows the popover, that a real key event is not
//! required to prove accepting a suggestion edits the field correctly
//! (`Composer::test_accept_recipient_suggestion` calls exactly what `Enter`
//! would), and that nothing shows without a candidate to offer.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use gtk::gdk;
use postio_gtk::composer::{self, RecipientCandidate};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::EmailAddress;

pub fn typing_a_prefix_offers_suggestions_and_accepting_one_completes_it() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    settle();

    // ── Nothing connected: typing shows nothing ──────────────────────────
    composer.test_set_to("grac");
    settle();
    assert!(!composer.test_recipient_popover_visible());

    // ── Connected, but no candidates for this prefix: still nothing ──────
    composer.connect_recipient_suggestions(|prefix| {
        if prefix == "grac" {
            vec![
                RecipientCandidate::Contact(EmailAddress::new(
                    Some("Grace Hopper"),
                    "grace@example.com",
                )),
                RecipientCandidate::Contact(EmailAddress::new(
                    Some("Graham Bell"),
                    "graham@example.net",
                )),
            ]
        } else {
            Vec::new()
        }
    });
    composer.test_set_to("zzzz");
    settle();
    assert!(!composer.test_recipient_popover_visible());

    // ── A prefix with candidates shows the popover ───────────────────────
    composer.test_set_to("grac");
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "grac should offer Grace and Graham"
    );

    // ── Accepting replaces only the token being typed ────────────────────
    assert!(composer.test_accept_recipient_suggestion());
    assert!(
        !composer.test_recipient_popover_visible(),
        "accepting closes it"
    );
    assert_eq!(
        composer.draft().to,
        vec![EmailAddress::new(Some("Grace Hopper"), "grace@example.com")],
        "the first suggestion is selected by default"
    );

    // ── …and leaves room to keep typing a second recipient ───────────────
    // `accept` already left "Grace Hopper <grace@example.com>, " in the
    // field (a full round trip of the first address, not just its raw
    // text), so typing a second prefix after it must not disturb the first.
    let existing = postio_model::address::format_list(&composer.draft().to);
    composer.test_set_to(&format!("{existing}, grac"));
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "completion still works for a second address after the first"
    );
    assert_eq!(
        composer.draft().to,
        vec![
            EmailAddress::new(Some("Grace Hopper"), "grace@example.com"),
            EmailAddress::new(None::<String>, "grac"),
        ],
        "the first address survived typing the start of a second"
    );
}

/// Revealing Cc and Bcc keeps the draft and the keyboard's place (FR-020).
///
/// The `+ Cc` button rebuilds nothing, but nothing said so. What it must not
/// do is what a naive "rebuild the header" would: lose a half-typed `To`, or
/// drop the keyboard somewhere the person did not put it.
///
/// Cc taking the keyboard afterwards *is* the behaviour — `show_copy_fields`
/// grabs it deliberately, because a field revealed and not focused is a field
/// the user must then reach for. What is asserted is that the address already
/// typed survives, and that the keyboard lands somewhere nameable rather than
/// nowhere.
pub fn revealing_cc_and_bcc_keeps_what_was_already_typed() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let composer = window.composer();
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    window.present();
    settle();

    composer.test_set_to("ada@example.com");
    composer.test_set_subject("the tide gate interlock");
    settle();

    composer.show_copy_fields();
    settle();

    assert_eq!(
        composer.draft().to.len(),
        1,
        "revealing Cc dropped the address already in To"
    );
    assert_eq!(
        composer.draft().to[0].address,
        "ada@example.com",
        "revealing Cc rewrote the address already in To"
    );
    assert_eq!(
        composer.test_subject(),
        "the tide gate interlock",
        "revealing Cc disturbed the subject"
    );
    assert_eq!(
        composer.focused_field(),
        Some(composer::Field::Cc),
        "the revealed field takes the keyboard, so the person can type into \
         the thing they just asked for"
    );
}

/// FR-020's other half: the rows can be put away again.
///
/// `show_copy_fields` had no inverse -- `cc_row.set_visible(false)` ran once
/// at construction and the `+ Cc` button hid itself the moment both rows were
/// up, so a person who opened Cc by accident carried it for the rest of the
/// draft. That is the dead end `/ux-architect` forbids.
///
/// What decides the shape is a rule already in this file at `resume`: Cc and
/// Bcc are visible *because* there is something in them. Hiding is therefore
/// only ever offered while they are empty, and the "hiding MUST NOT drop
/// addresses already entered" clause of FR-020 is satisfied by construction
/// rather than by remembering to special-case it -- there is nothing to drop.
/// The alternative, hiding a row that still holds addresses, is worse than
/// the dead end: those recipients would still be on the draft and still be
/// sent, by someone who can no longer see them.
///
/// So the verb is one toggle with an asymmetric half: put them away if they
/// are empty, otherwise take me to them. Never silently do nothing.
pub fn cc_and_bcc_put_themselves_away_only_while_they_are_empty() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let composer = window.composer();
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    window.present();
    settle();

    // ── Closed to begin with, and the way in is offered ──────────────────
    assert!(
        !composer.test_copy_fields_visible(),
        "a fresh draft shows neither Cc nor Bcc"
    );
    assert!(
        composer.test_more_button_visible(),
        "with the rows down, the control that raises them must be there, or \
         there is no way in at all"
    );

    composer.toggle_copy_fields();
    settle();
    assert!(
        composer.test_copy_fields_visible(),
        "the toggle did not raise Cc and Bcc"
    );
    assert!(
        !composer.test_more_button_visible(),
        "with the rows up, `+ Cc` has nothing left to offer and must go"
    );

    // ── Empty: the toggle is a way back out ──────────────────────────────
    composer.toggle_copy_fields();
    settle();
    assert!(
        !composer.test_copy_fields_visible(),
        "an empty Cc and Bcc must be dismissable, or opening them by accident \
         is permanent for the life of the draft"
    );
    assert!(
        composer.test_more_button_visible(),
        "dismissing the rows must bring back the control that raises them; \
         without it this is a one-way door in the other direction"
    );

    // ── Non-empty: it refuses, and says so by taking the keyboard there ──
    composer.toggle_copy_fields();
    settle();
    composer.test_set_cc("grace@example.com");
    settle();

    composer.toggle_copy_fields();
    settle();
    assert!(
        composer.test_copy_fields_visible(),
        "hiding a Cc that holds an address would leave a recipient on the \
         draft that the sender can no longer see -- it must refuse"
    );
    assert_eq!(
        composer.draft().cc.len(),
        1,
        "refusing to hide must not quietly clear the field instead"
    );
    assert_eq!(
        composer.draft().cc[0].address,
        "grace@example.com",
        "refusing to hide must not rewrite what is in the field"
    );
    assert_eq!(
        composer.focused_field(),
        Some(composer::Field::Cc),
        "a refusal that does nothing visible is indistinguishable from a bug; \
         the toggle takes the keyboard to what it would not put away"
    );
}
