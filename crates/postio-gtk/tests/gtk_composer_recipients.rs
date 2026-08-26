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

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::EmailAddress;

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn typing_a_prefix_offers_suggestions_and_accepting_one_completes_it() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-composer-recipients-{}", std::process::id()));
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
                EmailAddress::new(Some("Grace Hopper"), "grace@example.com"),
                EmailAddress::new(Some("Graham Bell"), "graham@example.net"),
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
