//! Resuming a draft picked out of the Drafts folder.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_composer.rs`.
//!
//! [`Composer::open`] deliberately refuses to replace what it is holding —
//! `c` a second time means "show me the draft", never "start another", which
//! is the one-composition-at-a-time rule. Resuming is the other request:
//! *this* draft, named, chosen out of a folder. It has to replace, and it may,
//! because a retained draft is autosaved and — since #166 — is itself a row in
//! that folder. Nothing is lost by swapping to another one and back.
//!
//! What this cannot prove is where the draft came from; that is `postio-app`'s
//! `tests/resume_draft.rs`, which activates a real row over a real store.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use gtk::gdk;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft, DraftId, EmailAddress};

fn a_draft(id: i64, subject: &str, to: &str) -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.id = DraftId::new(id);
    draft.subject = subject.to_owned();
    draft.to = vec![EmailAddress::new(None::<String>, to)];
    draft.body.text = Some(format!("About {subject}."));
    draft
}

pub fn resuming_replaces_the_draft_the_composer_was_holding() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

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
    let saves: std::rc::Rc<std::cell::RefCell<Vec<Draft>>> = Default::default();
    composer.connect_save({
        let saves = std::rc::Rc::clone(&saves);
        move |draft| saves.borrow_mut().push(draft.clone())
    });

    // Something in the composer, with an edit that has not been autosaved yet.
    composer.open(a_draft(1, "Tide gate interlock", "quinn@example.net"));
    settle();
    composer.test_set_subject("Tide gate interlock, revised");
    settle();
    assert!(
        saves.borrow().is_empty(),
        "the debounce has not elapsed; this is the edit resuming must not lose"
    );

    // ── `open` holds on, which is the rule resume is the exception to ────
    composer.open(a_draft(2, "Weir gauge", "grace@example.net"));
    settle();
    assert_eq!(
        composer.test_subject(),
        "Tide gate interlock, revised",
        "`c` a second time means show me the draft, never start another"
    );

    // ── resume replaces it, and flushes what was pending first ───────────
    composer.resume(a_draft(2, "Weir gauge", "grace@example.net"));
    settle();

    assert!(
        saves
            .borrow()
            .iter()
            .any(|draft| draft.subject == "Tide gate interlock, revised"),
        "swapping drafts must flush the pending edit rather than leave it in a \
         timer that is about to fire against the wrong draft"
    );
    assert_eq!(composer.test_subject(), "Weir gauge");
    assert!(composer.is_open());

    // ── and the one it swapped away from can be resumed back ─────────────
    composer.resume(a_draft(
        1,
        "Tide gate interlock, revised",
        "quinn@example.net",
    ));
    settle();
    assert_eq!(composer.test_subject(), "Tide gate interlock, revised");
}
