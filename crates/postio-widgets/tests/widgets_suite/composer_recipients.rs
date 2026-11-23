use crate::support_compose::Window;
use crate::support_compose::composer::{self, RecipientCandidate};
use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::EmailAddress;

pub fn typing_a_prefix_offers_suggestions_and_accepting_one_completes_it() {
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

    composer.test_set_to("grac");
    settle();
    assert!(!composer.test_recipient_popover_visible());

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

    composer.test_set_to("grac");
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "grac should offer Grace and Graham"
    );

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

    let existing = postio_model::address::format_list(&composer.draft().to);
    composer.test_set_to(&format!("{existing}, grac"));
    // A closed popover waits a few frames before it shows again (Completion::show_settled).
    crate::support_compose::settle_until("the second address's suggestions", || {
        composer.test_recipient_popover_visible()
    });
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

/// Cc taking the keyboard is the behaviour: `show_copy_fields` grabs it so a revealed field is not
/// one the user must then reach for.
pub fn revealing_cc_and_bcc_keeps_what_was_already_typed() {
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
