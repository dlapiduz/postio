use postio_widgets::onboarding::{Onboarding, Server, Settings, Status};

pub fn a_typed_name_reaches_the_submission_and_a_blank_one_stays_empty() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }

    let screen = Onboarding::new();
    screen.set_address("lena@example.com");
    screen.test_set_password("hunter2");
    screen.set_status(Status::Found(Settings {
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
        ..Settings::default()
    }));
    assert!(screen.can_submit(), "the form should be ready to submit");

    let submissions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    screen.connect_submit({
        let submissions = std::rc::Rc::clone(&submissions);
        move |submission| submissions.borrow_mut().push(submission.clone())
    });

    screen.submit();
    assert_eq!(
        submissions.borrow().last().unwrap().name,
        "",
        "nothing was typed, so the submission carries no name -- today's default"
    );

    screen.set_name("Ada Lovelace");
    screen.submit();
    assert_eq!(
        submissions.borrow().last().unwrap().name,
        "Ada Lovelace",
        "the name field's own text should reach the submission"
    );
}
