//! Return does the right thing in every onboarding field.
//!
//! `postio-68`: `onboarding.rs` wired `connect_activate` on exactly two
//! widgets, `address` and `password`. Fill in the five manual fields by hand
//! — the case that needs Return most, since autoconfig failed to spare you
//! the typing — and pressing it did nothing. The fix makes `Connect` the
//! window's default widget and gives the five entries `activates-default`,
//! so `Ret` reaches it the same way it would in any dialog.
//!
//! `#629` is the same bug recurring on a field that did not exist when
//! `postio-68` was fixed: onboarding's "Your name" field (#603) never got a
//! `connect_activate` at all, so Return in the very first field of the form
//! did nothing. This file now drives Return through every field the form
//! has — name, address, password, and the five manual fields — in one
//! place, precisely so the *next* field added here cannot silently repeat
//! this a third time.
//!
//! Its own file with one test function: GTK is initialised once per process,
//! from one thread, and a second `#[test]` here would race `adw::init()` —
//! see the note in `gtk_onboarding.rs`.

use crate::settle;
use adw::prelude::*;
use postio_gtk::onboarding::{Onboarding, Server, Settings, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

pub fn return_does_the_right_thing_in_every_field() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gtk::gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let screen = Onboarding::new();
    window.set_content(Some(&screen));
    settle();

    // ── name: nothing to probe or submit yet, so Return moves on ──────────
    assert!(
        !screen.test_address_has_focus(),
        "sanity: nothing has claimed the address field's focus yet"
    );
    screen.test_activate_name();
    settle();
    assert!(
        screen.test_address_has_focus(),
        "Return in the name field should move the keyboard to the address \
         field, the same way Tab already does (#629)"
    );

    // ── address: Return asks for a probe ───────────────────────────────────
    let probed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    screen.connect_probe({
        let probed = std::rc::Rc::clone(&probed);
        move |address| probed.borrow_mut().push(address.to_owned())
    });
    screen.set_address("lena@example.com");
    screen.test_activate_address();
    settle();
    assert_eq!(
        *probed.borrow(),
        ["lena@example.com".to_string()],
        "Return in the address field should still probe it"
    );

    let submissions = std::rc::Rc::new(std::cell::Cell::new(0));
    screen.connect_submit({
        let submissions = std::rc::Rc::clone(&submissions);
        move |_| submissions.set(submissions.get() + 1)
    });

    // ── password: Return submits ───────────────────────────────────────────
    // A guess that opens and fills the manual form — exactly the state a
    // failed autoconfig leaves the screen in, and the case `postio-68` bit
    // hardest: everything typed by hand, then Return does nothing.
    screen.test_set_password("hunter2");
    screen.set_status(Status::Manual {
        suggestion: Some(Settings {
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
        }),
    });
    settle();
    assert!(screen.manual_shown());
    assert!(screen.can_submit(), "the form should be ready to submit");

    screen.test_activate_password();
    settle();
    assert_eq!(
        submissions.get(),
        1,
        "Return in the password field should still submit"
    );

    // ── the five manual fields: Return submits from each of them too ──────
    screen.test_activate_manual_fields();
    settle();

    assert_eq!(
        submissions.get(),
        6,
        "Return in a manual field should submit, same as address or password"
    );
}
