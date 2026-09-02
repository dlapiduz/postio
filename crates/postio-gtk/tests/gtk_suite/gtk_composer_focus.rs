//! `postio-43`: the composer's focus retry, forced through its unmapped path.
//!
//! `composer.rs::focus_first` grabs focus on a widget that has just become
//! visible and, per its own doc comment, is not yet mapped on that frame —
//! so a failed grab is retried once layout has run. The retry used to be
//! scheduled with `idle_add_local_once`, which is not ordered against the
//! frame clock driving that layout pass: whichever ran first won, same shape
//! as the drill-in scroll race `postio-1ff` turned out to be (`8daa510`).
//!
//! Opening the composer before the window is ever presented guarantees the
//! precondition deterministically — nothing in the tree is realized, so the
//! first `grab_focus` cannot succeed — where relying on real desktop timing
//! left it "not reproduced" (see the issue body). What is asserted here is
//! the retry actually landing once real frames are pumped afterward.
//!
//! Skips without a display. Nothing here touches the network.

use std::time::{Duration, Instant};

use gtk::gdk;
use postio_gtk::composer::{self, Field};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::{AccountId, Draft, DraftKind, EmailAddress, MessageBody};

fn started(kind: DraftKind) -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.kind = kind;
    draft.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    draft.subject = "the mbox importer".to_owned();
    draft.body = MessageBody {
        text: Some("Looking now.".to_owned()),
        html: None,
    };
    draft
}

/// Waits for `done`, driving both idle and frame-clock sources — a genuine
/// regression still fails when the deadline runs out, a slow machine only
/// makes it slower. Modelled on `gtk_thread.rs::settle_until`.
fn settle_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_millis(3000);
    while !done() && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

pub fn focus_lands_when_the_composer_opens_before_the_window_is_ever_mapped() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let composer = composer::install(&window);

    // Nothing has been presented yet, so nothing in the tree is realized —
    // the very condition `focus_first`'s doc comment describes, forced
    // rather than hoped for.
    composer.open(started(DraftKind::New));

    window.present();
    settle_until(|| composer.focused_field().is_some());

    assert_eq!(
        composer.focused_field(),
        Some(Field::To),
        "new mail starts in To, once the retry has had real frames to land in"
    );

    // -- and a reply, which starts in the body instead ---------------------

    let window = Window::default();
    let composer = composer::install(&window);
    composer.open(started(DraftKind::Reply));
    window.present();
    settle_until(|| composer.focused_field().is_some());

    assert_eq!(
        composer.focused_field(),
        Some(Field::Body),
        "a reply starts in the body, once the retry has had real frames to land in"
    );
}
