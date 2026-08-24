//! Return submits onboarding from the manual server fields too.
//!
//! `postio-68`: `onboarding.rs` wired `connect_activate` on exactly two
//! widgets, `address` and `password`. Fill in the five manual fields by hand
//! — the case that needs Return most, since autoconfig failed to spare you
//! the typing — and pressing it did nothing. The fix makes `Connect` the
//! window's default widget and gives the five entries `activates-default`,
//! so `Ret` reaches it the same way it would in any dialog.
//!
//! Its own file with one test function: GTK is initialised once per process,
//! from one thread, and a second `#[test]` here would race `adw::init()` —
//! see the note in `gtk_onboarding.rs`.

use adw::prelude::*;
use postio_gtk::onboarding::{Onboarding, Server, Settings, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn enter_in_a_manual_field_submits() {
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

    // A guess that opens and fills the manual form — exactly the state a
    // failed autoconfig leaves the screen in, and the case the bug bit
    // hardest: everything typed by hand, then Return does nothing.
    screen.set_address("lena@example.com");
    screen.test_set_password("hunter2");
    screen.set_status(Status::Manual {
        suggestion: Some(Settings {
            imap: Server {
                host: "imap.example.com".to_owned(),
                port: 993,
                tls: true,
            },
            smtp: Server {
                host: "smtp.example.com".to_owned(),
                port: 465,
                tls: true,
            },
            login: "lena@example.com".to_owned(),
            source: "a common-name guess".to_owned(),
            ..Settings::default()
        }),
    });
    settle();
    assert!(screen.manual_shown());
    assert!(screen.can_submit(), "the form should be ready to submit");

    let submissions = std::rc::Rc::new(std::cell::Cell::new(0));
    screen.connect_submit({
        let submissions = std::rc::Rc::clone(&submissions);
        move |_| submissions.set(submissions.get() + 1)
    });

    screen.test_activate_manual_fields();
    settle();

    assert_eq!(
        submissions.get(),
        5,
        "Return in a manual field should submit, same as address or password"
    );
}
