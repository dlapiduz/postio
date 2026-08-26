//! `e`/`E`/`f` open a reply, reply-all or forward of whatever the reading
//! pane says is on screen — on a real display, since the wiring runs through
//! [`postio_core::CommandId`] dispatch and a real key press.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.
//!
//! `reply_draft`'s own mapping (which [`postio_model::reply`] function each
//! command reaches) is unit-tested in `src/composer.rs` with no display; what
//! needs one here is that the key actually reaches
//! [`Composer::connect_reply_source`] and opens the composer with the right
//! draft, and that nothing happens without a source connected or while the
//! composer is already open.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Account, AccountId, EmailAddress, Message};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

fn press_shifted(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::SHIFT_MASK,
    );
    settle();
}

fn a_message() -> Message {
    let mut message = Message::new(
        AccountId::new(1),
        postio_model::ids::MailboxId::new(1),
        chrono::Utc::now(),
    );
    message.id = postio_model::ids::MessageId::new(7);
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
    message.subject = Some("Quarterly numbers".to_owned());
    message
}

fn an_account() -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "grace@example.com"),
    );
    account.id = AccountId::new(1);
    account
}

#[test]
fn e_shift_e_and_f_open_reply_reply_all_and_forward() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-composer-reply-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
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

    // ── Nothing connected: the key does nothing, not a broken composer ───
    press(&window, "e");
    assert!(
        !composer.is_open(),
        "e with no reply source connected must not open a blank composer"
    );

    // ── A source with nothing to offer: also nothing ─────────────────────
    composer.connect_reply_source(|| None);
    press(&window, "e");
    assert!(!composer.is_open());

    // ── A real source: e replies ──────────────────────────────────────────
    composer.connect_reply_source(|| Some((a_message(), an_account())));
    press(&window, "e");
    assert!(composer.is_open(), "e opens a reply");
    let draft = composer.draft();
    assert_eq!(draft.subject, "Re: Quarterly numbers");
    assert_eq!(
        draft.to,
        vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
        "a plain reply goes to the sender"
    );
    composer.discard();
    settle();

    // ── Shift+E reply-alls ─────────────────────────────────────────────
    press_shifted(&window, "e");
    assert!(composer.is_open(), "shift+e opens reply-all");
    assert_eq!(composer.draft().subject, "Re: Quarterly numbers");
    composer.discard();
    settle();

    // ── f forwards ─────────────────────────────────────────────────────
    press(&window, "f");
    assert!(composer.is_open(), "f opens a forward");
    assert_eq!(composer.draft().subject, "Fwd: Quarterly numbers");
    assert!(
        composer.draft().to.is_empty(),
        "a forward starts with no recipients"
    );

    // ── Already composing: e does not reopen or reset it ─────────────────
    // The forward from the step above is still open; e is for the reading
    // pane, not for replacing an in-progress composition with a new one --
    // but it must not go silently ignored either (#426). The status line
    // was already showing something ("draft is in the composer only", set
    // when the forward opened), so the test is not "status is non-empty"
    // but "status changed to say why e did nothing".
    press(&window, "e");
    assert_eq!(
        composer.draft().subject,
        "Fwd: Quarterly numbers",
        "e while composing must not clobber what is being written"
    );
    assert_eq!(
        composer.status(),
        "not opened — finish or close the current draft first",
        "e while composing silently did nothing instead of explaining why: {:?}",
        composer.status()
    );
}
