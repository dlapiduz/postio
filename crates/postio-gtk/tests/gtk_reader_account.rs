//! The reading pane names the account a message arrived in. #185.
//!
//! ADR 0005 Q4 first put per-account identity on every list row in unified
//! scope. The maintainer's call on #185 is that a mixed list is fine *as* a
//! mixed list — you asked for all your mail, so of course it is mixed — and
//! "whose is this?" is a question about the message in front of you rather
//! than about forty rows at once. So it is answered here, once, and the row's
//! 3px left edge goes on meaning `selected` and nothing else.
//!
//! One test function: GTK is single-threaded and initialised once per binary.
//! Without a display it skips. Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use chrono::{TimeZone, Utc};
use gtk::gdk;
use postio_gtk::reader::MessageHeader;
use postio_gtk::{app, fonts, style};
use postio_model::address::EmailAddress;

#[test]
fn the_header_names_the_account_only_when_there_is_more_than_one() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let header = MessageHeader::new();
    header.set_message(
        &[EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
        &[EmailAddress::new(None::<String>, "you@example.net")],
        &[],
        Some("Quarterly report"),
        Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap(),
    );

    // ── one account: no trace of it ─────────────────────────────────────
    //
    // Somebody who has never configured a second account must see nothing
    // about accounts at all. Naming the only one there is would be noise
    // about a choice they have not made.
    header.set_account(None, 0);
    assert_eq!(
        header.account_label(),
        None,
        "the account line is drawn in a single-account install, where it can \
         only ever say the obvious"
    );

    // ── more than one: the pane says which ──────────────────────────────
    header.set_account(Some("Work"), 2);
    assert_eq!(header.account_label().as_deref(), Some("Work"));

    // ── and it follows the message, rather than latching ────────────────
    header.set_account(Some("Home"), 5);
    assert_eq!(
        header.account_label().as_deref(),
        Some("Home"),
        "the line has to follow the message the pane is showing; a stale \
         account name is worse than none, because it is believed"
    );

    // ── clearing the pane clears it too ─────────────────────────────────
    header.clear();
    assert_eq!(
        header.account_label(),
        None,
        "the pane closed, so the account of the message that was in it is no \
         longer a fact about anything on screen"
    );
}
