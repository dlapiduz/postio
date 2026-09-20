//! An oversize message is refused before anything is queued (spec 002,
//! FR-055 and FR-056).
//!
//! `postio_model::size` decides the arithmetic and is unit-tested there. What
//! needs a display is the part that was missing entirely: that the composer
//! *asks*, that the refusal reaches the person, and — the assertion that
//! matters — that the send handler is never called. A size check that
//! computes the right number and lets the draft through anyway is the shape
//! of bug this repository has most of: two layers that each pass and are not
//! joined up.
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

/// A draft addressed to somebody, carrying one part of `size` bytes.
fn draft_carrying(size: u64, name: &str) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
    draft.subject = "the tide gate footage".to_owned();
    draft.body.text = Some("Attached.".to_owned());
    let mut part = Attachment::new(MessageId::UNASSIGNED, "video/mp4", size);
    part.filename = Some(name.to_owned());
    draft.attachments = vec![part];
    draft
}

pub fn an_oversize_draft_is_refused_before_it_reaches_the_send_handler() {
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

    // ── Over the limit: refused, and the handler never runs ──────────────
    composer.set_size_limit(Some(25_000_000));
    composer.open(draft_carrying(30_000_000, "footage.mp4"));
    settle();
    composer.send();
    settle();

    assert!(
        sent.borrow().is_empty(),
        "the draft reached the send handler anyway, so the check computes a \
         number and changes nothing"
    );
    assert!(
        composer.is_open(),
        "a refused send must leave the composer open -- closing it would \
         hand the person a problem and take away the surface for fixing it"
    );

    let status = composer.status();
    assert!(
        status.contains("footage.mp4"),
        "the refusal does not name what is taking up the room, so the person \
         is left guessing which part to remove: {status}"
    );
    assert!(
        status.contains("25"),
        "the refusal does not name the limit: {status}"
    );

    // ── Under the limit: sends ───────────────────────────────────────────
    //
    // Closed first: the refusal deliberately left the composer open, and
    // `open` will not clobber a draft that is still being edited.
    composer.close();
    settle();
    composer.open(draft_carrying(1_000_000, "clip.mp4"));
    settle();
    composer.send();
    settle();
    assert_eq!(
        sent.borrow().len(),
        1,
        "a draft under the limit must send: {}",
        composer.status()
    );

    // ── No configured limit: checks nothing ──────────────────────────────
    //
    // The decision recorded for T039. A guessed ceiling refuses mail the
    // provider would have taken, and the person has no way to tell Postio's
    // opinion from their provider's rule.
    composer.set_size_limit(None);
    composer.close();
    settle();
    composer.open(draft_carrying(500_000_000, "enormous.mp4"));
    settle();
    composer.send();
    settle();
    assert_eq!(
        sent.borrow().len(),
        2,
        "an unconfigured limit became a guessed one: {}",
        composer.status()
    );
}
