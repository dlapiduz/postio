//! The two things the composer asks about before doing them (spec 002,
//! FR-057 and FR-061).
//!
//! `postio_model::mention` decides whether the words are there and is unit
//! tested where it lives. What needs a display is the join: that `send`
//! actually asks, which shows up here as the send handler *not* being called
//! and the composer staying open. A check that computes the right answer and
//! lets the draft through anyway is the characteristic bug in this
//! repository, and it is invisible to the model's own tests.
//!
//! What is **not** asserted is the dialog's buttons. Driving an
//! `adw::AlertDialog` headlessly is not something this suite can do -- the
//! discard dialog has never been display-tested either -- so the response
//! handler is covered by inspection and the observable half is covered here.
//! Stated rather than left as a gap somebody has to rediscover.
//!
//! One test function for both, and deliberately: each needs a real composer
//! and a real composer is a `WebView`, which is the resource `gtk_suite` is
//! already short of (#957). Two assertions about the same question do not
//! need two processes.
//!
//! One test function: GTK is single-threaded and initialised once.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. These tests set it before the app under test
// starts, which is the one moment it is sound.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::MessageId;
use postio_model::{AccountId, Attachment, Draft, EmailAddress};

use crate::settle;

fn draft_saying(text: &str) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
    draft.subject = "the tide gate report".to_owned();
    draft.body.text = Some(text.to_owned());
    draft
}

pub fn the_composer_asks_before_the_two_things_it_cannot_take_back() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

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

    // ── Says it, carries nothing: does not reach the handler ─────────────
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

    // ── Says it and carries it: sends, with nothing to ask ───────────────
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

    // ── Says nothing of the kind: sends ──────────────────────────────────
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

    // ── FR-061: discarding asks, unless there is nothing to lose ─────────
    //
    // The one composer action with no way back -- a queued send can still be
    // cancelled from Drafts, and everything else is a keystroke away from
    // being retyped. As with the dialog above, what is asserted is the
    // observable half: the draft is still there afterwards, because the
    // answer has not been given yet.
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

    // An empty draft is dropped rather than defended. Asking "discard this?"
    // about nothing is a dialog that can only ever be answered one way, and
    // teaching someone to dismiss it unread is how the dialog above stops
    // working too.
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
