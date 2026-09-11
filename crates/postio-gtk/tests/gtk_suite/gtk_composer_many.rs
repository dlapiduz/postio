//! One in the pane, many in windows (ADR 0034; spec 002, FR-010, FR-011,
//! FR-013).
//!
//! The behaviour this replaces was quiet, which is what made it worth
//! replacing: asking to compose while already composing focused the field you
//! were already in, so the key looked broken rather than declined. FR-011
//! forbids all three of the old possibilities by name — *"MUST move the
//! pane's draft into a detached window rather than refusing, discarding it,
//! or asking"*.
//!
//! What the composer used to rely on is gone with it. Detaching kept
//! everything because the widget was reparented and never rebuilt; a second
//! composer cannot inherit that, so what makes the move lossless is now the
//! thing ADR 0004 already decided — the draft is the record, and moving one
//! between surfaces is a save and a resume. `gtk_composer_resume.rs` is what
//! proves that path carries text, formatting, recipients and attachments.
//!
//! One test function: GTK is single-threaded and initialised once.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. These tests set it before the app under test
// starts, which is the one moment it is sound.

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft, DraftId};

use crate::settle;

fn a_draft(id: i64, subject: &str) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.id = DraftId::new(id);
    draft.subject = subject.to_owned();
    draft.body.text = Some(format!("About {subject}."));
    draft
}

pub fn a_second_draft_moves_the_first_into_a_window_of_its_own() {
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
    window.present();

    // ── The pane holds the first ─────────────────────────────────────────
    let first = window.open_draft(a_draft(1, "the weir gauge"));
    settle();
    assert!(first.is_open(), "the first draft did not open at all");
    assert!(
        !first.is_detached(),
        "the first draft should be in the pane, not a window"
    );

    // ── The second pushes it out rather than refusing ────────────────────
    let second = window.open_draft(a_draft(2, "the tide gate"));
    settle();

    assert_eq!(
        second.test_subject(),
        "the tide gate",
        "asking for a second draft gave back the first, which is the quiet \
         refusal FR-011 forbids"
    );
    assert!(
        !second.is_detached(),
        "the new draft belongs in the pane; it is the old one that moves"
    );
    assert!(
        first.is_detached(),
        "the first draft was not moved into a window of its own"
    );
    assert!(
        first.is_open(),
        "the first draft was closed rather than moved, which is the \
         discarding FR-011 forbids"
    );
    assert_eq!(
        first.test_subject(),
        "the weir gauge",
        "the first draft lost its subject on the way to its own window"
    );
    assert_eq!(
        first.draft().body.text.as_deref(),
        Some("About the weir gauge."),
        "the first draft lost its text on the way to its own window, which is \
         the one thing this whole arrangement exists to prevent"
    );

    // ── FR-013: asking for one already open brings it forward ────────────
    let again = window.open_draft(a_draft(1, "the weir gauge"));
    settle();
    assert!(
        again.is_detached(),
        "a draft that is already open in a window must be brought forward, \
         not opened a second time in the pane"
    );
    assert_eq!(
        again.draft().id,
        DraftId::new(1),
        "asking for draft 1 produced a different composer"
    );
    assert_eq!(
        second.test_subject(),
        "the tide gate",
        "bringing the first forward disturbed the one in the pane"
    );
}
