use crate::support_compose::Window;
use crate::support_compose::composer::{self, RecipientCandidate};
use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::EmailAddress;

pub fn typing_a_prefix_offers_suggestions_and_accepting_one_completes_it() {
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

/// Cc and Bcc are visible because something is in them, so hiding is offered only while they are
/// empty and never drops addresses; otherwise the toggle takes the person to them.
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
