use postio_widgets::onboarding::{Onboarding, Server, Settings, Status};

pub fn a_guess_fills_the_manual_form_and_opens_it() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }

    // What `guess_common_names` produces: convention, not a lookup, so `source` must not read
    // "entered by hand".
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

    let settings = screen.settings();
    assert_eq!(settings.imap.host, "imap.example.com");
    assert_eq!(settings.imap.port, 993);
    assert_eq!(settings.smtp.host, "smtp.example.com");
    assert_eq!(settings.smtp.port, 465);
    assert_eq!(settings.login, "lena@example.com");

    // Open, not merely filled: a guess is what needs correcting, and `Status::Manual` makes the
    // heading say nothing was published.
    assert!(
        screen.manual_shown(),
        "the guess filled the server fields and left them hidden"
    );
    assert!(
        !matches!(screen.status(), Status::Found(_)),
        "a guess must never be presented as a discovery"
    );
}
