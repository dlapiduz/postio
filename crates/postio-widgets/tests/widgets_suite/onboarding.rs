use postio_widgets::onboarding::{Onboarding, Server, Settings, Status};

pub fn a_repair_arrives_with_the_address_and_the_servers_already_filled_in() {
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

    // Empty here would mean the user retypes known servers, and the submit overwrites the account
    // row with blanks.
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

    assert!(
        !screen.manual_shown(),
        "the repair opened the manual server form it had no reason to"
    );
}

pub fn a_disabled_browser_sign_in_says_what_it_waits_for() {
    let found = Settings {
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
        source: "the provider table".to_owned(),
        oauth_sign_in: true,
        ..Settings::default()
    };
    let screen = Onboarding::new();
    screen.set_address("lena@example.com");
    screen.set_status(Status::Found(found));
    assert!(!screen.can_submit());
    let hint = screen
        .test_submit_hint()
        .expect("a disabled button gives its reason");
    assert!(hint.contains("OAuth client ID"), "{hint:?}");
    screen.test_set_oauth_client_id("1234.apps.example.com");
    assert!(screen.can_submit());
    assert_eq!(screen.test_submit_hint(), None);
}
