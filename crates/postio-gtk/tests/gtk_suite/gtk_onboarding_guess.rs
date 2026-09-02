//! The first-run screen when nothing authoritative answered.
//!
//! `postio-69`: a domain that publishes no autoconfig — every custom domain,
//! which is exactly the person least able to answer — got five empty boxes.
//! `postio-app` now runs the probe with `guess_common_names` on, so the
//! report carries an `imap.<domain>` / `smtp.<domain>` suggestion. This is
//! the other end of that: the suggestion has to *arrive in the form*, and it
//! has to arrive as a starting point rather than as a discovery.
//!
//! Its own file with one test function: GTK is initialised once, per process,
//! from one thread. Two `#[test]`s in a file race `adw::init()` and the loser
//! takes the no-display branch and passes without asserting anything — see
//! the note in `gtk_onboarding.rs`.

use postio_gtk::onboarding::{Onboarding, Server, Settings, Status};

pub fn a_guess_fills_the_manual_form_and_opens_it() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }

    // What `guess_common_names` produces: convention, not a lookup. Note
    // `source` — the card heading reads from it, and "entered by hand" would
    // be a lie about where these came from.
    let guess = Settings {
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
        login: "lena@example.com".to_owned(),
        source: "a common-name guess".to_owned(),
        ..Settings::default()
    };

    let screen = Onboarding::new();
    screen.set_address("lena@example.com");
    screen.set_status(Status::Manual {
        suggestion: Some(guess),
    });

    // The boxes are no longer empty. This is the whole of #69: without it the
    // user is asked for five values they have no way to know.
    let settings = screen.settings();
    assert_eq!(settings.imap.host, "imap.example.com");
    assert_eq!(settings.imap.port, 993);
    assert_eq!(settings.smtp.host, "smtp.example.com");
    assert_eq!(settings.smtp.port, 465);
    assert_eq!(settings.login, "lena@example.com");

    // Open, not merely filled: a prefilled form the user cannot see is a form
    // they cannot correct, and a guess is exactly the thing that needs
    // correcting. `Status::Manual` is what makes this honest — the heading
    // says nothing was published, so the fields read as a starting point
    // rather than as something Postio looked up.
    assert!(
        screen.manual_shown(),
        "the guess filled the server fields and left them hidden"
    );
    assert!(
        !matches!(screen.status(), Status::Found(_)),
        "a guess must never be presented as a discovery"
    );
}
