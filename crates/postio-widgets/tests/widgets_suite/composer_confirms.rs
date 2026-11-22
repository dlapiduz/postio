use std::cell::RefCell;
use std::rc::Rc;

use crate::support_compose::Window;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use postio_model::ids::MessageId;
use postio_model::{AccountId, Attachment, Draft, EmailAddress};

use crate::support_compose::settle;

fn draft_saying(text: &str) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
    draft.subject = "the tide gate report".to_owned();
    draft.body.text = Some(text.to_owned());
    draft
}

pub fn the_composer_asks_before_the_two_things_it_cannot_take_back() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    let composer = window.composer();
    window.present();

    let sent: Rc<RefCell<Vec<Draft>>> = Rc::new(RefCell::new(Vec::new()));
    composer.connect_send({
        let sent = sent.clone();
        move |draft| sent.borrow_mut().push(draft.clone())
    });

    composer.open(draft_saying("Please find attached the tide gate report."));
    settle();
    composer.send();
    settle();

    assert!(
        sent.borrow().is_empty(),
        "the message went out claiming an attachment it does not carry, so \
         the check computes an answer and changes nothing"
    );
    assert!(
        composer.is_open(),
        "the composer closed, which takes away the surface for adding the \
         file the person was just asked about"
    );

    let mut carrying = draft_saying("Please find attached the tide gate report.");
    let mut part = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 2_048);
    part.filename = Some("report.pdf".to_owned());
    carrying.attachments = vec![part];

    composer.close();
    settle();
    composer.open(carrying);
    settle();
    composer.send();
    settle();

    assert_eq!(
        sent.borrow().len(),
        1,
        "a message that carries what it claims must send without a word: {}",
        composer.status()
    );

    composer.close();
    settle();
    composer.open(draft_saying(
        "Morning — the gate is armed and the tide is out.",
    ));
    settle();
    composer.send();
    settle();

    assert_eq!(
        sent.borrow().len(),
        2,
        "an ordinary message was held up: {}",
        composer.status()
    );

    // Discarding is the one composer action with no way back. Only the observable half is asserted:
    // the draft is still there.
    composer.close();
    settle();
    composer.open(draft_saying("Half a sentence nobody wants to lose."));
    settle();
    composer.request_discard();
    settle();

    assert!(
        composer.is_open(),
        "a draft with writing in it was discarded without asking, which is \
         the one thing in the composer that cannot be undone"
    );
    assert_eq!(
        composer.test_subject(),
        "the tide gate report",
        "the draft was cleared while the question was still on screen"
    );

    // An empty draft is dropped rather than defended: a discard prompt about nothing teaches people
    // to dismiss it unread.
    composer.close();
    settle();
    let mut blank = Draft::new(AccountId::new(1));
    blank.subject = String::new();
    composer.open(blank);
    settle();
    composer.request_discard();
    settle();
    assert!(
        !composer.is_open(),
        "an empty draft was defended with a question that has one answer"
    );
}
