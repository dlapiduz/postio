//! The first-run screen when it is not a first run.
//!
//! `postio-67`: an account whose password the keyring will not give up is
//! sent back to this screen instead of into a window that cannot sync. That
//! second visit is a *repair*, and it has to arrive knowing what the account
//! row already knows — the address and the servers — so the user supplies
//! the one thing that is missing.
//!
//! Its own file with one test function: GTK is single-threaded and
//! initialised once, and `gtk_toast.rs` explains what a crate that inits it
//! from a unit-test thread pool does to CI. Two `#[test]`s here would race
//! `adw::init()`, and the loser would take the no-display branch and pass
//! without asserting anything — a test that cannot fail.
//!
//! What the screen *says* is checked in the crate's own unit tests instead:
//! `Status::message` is pure, and wording needs no display.

use postio_gtk::onboarding::{Onboarding, Server, Settings, Status};

#[test]
fn a_repair_arrives_with_the_address_and_the_servers_already_filled_in() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }

    // The servers a configured account already has, as `postio-app` reads
    // them off the row it is about to ask for a password for.
    let known = Settings {
        imap: Server {
            host: "imap.example.com".to_owned(),
            port: 993,
            security: Default::default(),
        },
        smtp: Server {
            host: "smtp.example.com".to_owned(),
            port: 465,
            security: Default::default(),
        },
        login: "lena@example.net".to_owned(),
        source: "saved with this account".to_owned(),
        ..Settings::default()
    };

    let screen = Onboarding::new();
    screen.set_address("lena@example.com");
    screen.set_status(Status::Reauthenticate(known));

    // What the screen would submit, which is what gets written back. Empty
    // here means the user retypes servers Postio already knows — and the
    // submit overwrites the account row with the blanks.
    let settings = screen.settings();
    assert_eq!(settings.imap.host, "imap.example.com");
    assert_eq!(settings.imap.port, 993);
    assert_eq!(settings.smtp.host, "smtp.example.com");
    assert_eq!(settings.smtp.port, 465);
    assert_eq!(
        settings.login, "lena@example.net",
        "the login is not always the address, and a repair must not lose it"
    );
    assert_eq!(screen.address(), "lena@example.com");

    // The servers are known good, so the form that exists for filling them
    // in by hand stays out of the way. A probe that found nothing opens it;
    // a repair has nothing to look for.
    assert!(
        !screen.manual_shown(),
        "the repair opened the manual server form it had no reason to"
    );
}
