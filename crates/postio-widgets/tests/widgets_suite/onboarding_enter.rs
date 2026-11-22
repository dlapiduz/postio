use crate::support_compose::settle;
use crate::support_compose::{app, fonts, style};
use adw::prelude::*;
use postio_widgets::onboarding::{Onboarding, Server, Settings, Status};

pub fn return_does_the_right_thing_in_every_field() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gtk::gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = adw::Window::new();
    window.present();
    settle();

    let screen = Onboarding::new();
    window.set_content(Some(&screen));
    settle();

    screen.focus_name();
    settle();
    assert!(
        screen.test_name_has_focus(),
        "a fresh onboarding form must start in its first field; opening in \
         the address leaves the name above it looking already dealt with"
    );

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

    screen.test_activate_manual_fields();
    settle();

    assert_eq!(
        submissions.get(),
        6,
        "Return in a manual field should submit, same as address or password"
    );
}
