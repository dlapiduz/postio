//! A body that did not decode cleanly says so in the pane (#901).
//!
//! `ParsedMessage::encoding_problems` was computed and read by nothing: the
//! one signal Postio has that the words on screen are not the words that were
//! sent, dropped at `into_message` and never given a column, an event or a
//! surface. Three degradations rode on it — base64 outside its alphabet
//! arriving as raw base64 text, an unknown `Content-Transfer-Encoding` shown
//! verbatim per RFC 2045 §6.4, a charset that lost octets to U+FFFD. Each is
//! the right degradation and each is indistinguishable from a message that
//! simply said that.
//!
//! # Why this is in `app_suite` and not a unit test
//!
//! Because the issue asks for exactly that, and says why: *"a test asserts
//! the fixture that triggers it produces the surface, not merely the flag —
//! a test on the flag alone is the bug again."* The flag was always
//! computable; what it never had was a path. So this drives the whole one —
//! parse, store, open the message the way a person does, and look at the
//! reading pane — and would fail if any link between them dropped it, which
//! is what every link did until now.
//!
//! The control is the other half. A clean message must draw no caveat: a
//! warning that appears on mail that decoded perfectly is one people learn to
//! ignore, and it would also let this file pass with `set_visible(true)`
//! wired to nothing in particular.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe. Set before the app under test
// starts, which is the one moment it is sound; the library forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{BlobStore, Database, test_support};

/// Latin-1 octets under a `charset=utf-8` header: the mojibake a user
/// actually reports, and the direction of it that cannot be undone. `é`
/// arrives as `0xe9`, which is not valid UTF-8, so the decode substitutes
/// U+FFFD and the original byte is gone for good — out of the stored body,
/// out of the search index, and out of every reply that quotes it.
const LOSSY: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Summer plans\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
L'\xe9t\xe9 was the summer everything happened\r\n";

/// The same message, honestly labelled. Nothing here should draw a caveat.
const CLEAN: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Winter plans\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
The winter was quieter\r\n";

/// Store `raw` as a message with a body, the way the backfill commits one.
fn store(
    database: &Database,
    account: postio_model::ids::AccountId,
    mailbox: postio_model::ids::MailboxId,
    raw: &[u8],
    subject: &str,
    received: chrono::DateTime<chrono::Utc>,
) -> postio_model::ids::MessageId {
    let connection = database.connection().expect("a connection");
    let repository = MessageRepository::new(&connection);
    let parsed = postio_model::mime::parse(raw);

    let mut message = Message::new(account, mailbox, received);
    message.subject = Some(subject.to_owned());
    message.sync.body_state = BodyState::Full;
    let id = repository.create(&mut message).expect("a message");

    let stored = StoredBody {
        text: parsed.body.text.clone(),
        html: parsed.body.html.clone(),
        headers: None,
        headers_truncated: false,
        // From the parser, never hand-set: a test that asserted the surface
        // over a flag it wrote itself would prove the surface works and
        // nothing about whether anything ever sets it.
        encoding_problems: parsed.encoding_problems,
    };
    repository
        .set_body(id, &stored, BodyState::Full)
        .expect("a body");
    id
}

pub fn a_body_that_did_not_decode_cleanly_says_so_in_the_pane() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        drop(connection);
        // Newest first, so the list opens on the lossy one and the control is
        // one `j` away.
        store(
            &database,
            account.id,
            inbox,
            CLEAN,
            "Winter plans",
            chrono::Utc::now() - chrono::Duration::hours(1),
        );
        store(
            &database,
            account.id,
            inbox,
            LOSSY,
            "Summer plans",
            chrono::Utc::now(),
        );
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() == 2),
        "the two messages never reached the list"
    );

    // ── the lossy one draws the caveat ───────────────────────────────────
    activate_first_row(&window);
    assert!(
        settle_until(|| window.reading()),
        "the message was opened and the reading pane never filled"
    );
    assert!(
        settle_until(|| window.reader().shows_encoding_problems()),
        "a body that decoded to U+FFFD drew no caveat, so the reader is \
         presenting a guess as the sender's words — which is what #901 is"
    );

    // ── and the clean one does not ───────────────────────────────────────
    // The control, and the reason the assertion above can fail: without it a
    // notice pinned visible would satisfy this file.
    //
    // Waited for the *repaint*, not for the caveat to go away. `settle_until`
    // on a negation returns the moment it holds, and it holds during the
    // transition -- `render` clears the notice before the new body is drawn
    // -- so a version of this that waited for `!shows_encoding_problems`
    // would pass without the clean message ever reaching the pane, and would
    // go on passing if the caveat were never set again for anything.
    let painted = window.reader().paints();
    window.list().next_row();
    assert!(
        settle_until(|| window.reader().paints() > painted),
        "moving to the second message never repainted the reading pane, so \
         the assertion below would be about the first one"
    );
    assert!(
        !window.reader().shows_encoding_problems(),
        "a message that decoded cleanly is carrying a decode caveat; a \
         warning that is sometimes wrong is one people learn to ignore"
    );

    bridge.shutdown();
}

fn activate_first_row(window: &Window) {
    let view = find_list_view(window.upcast_ref::<gtk::Widget>())
        .expect("the message list is built on a GtkListView");
    view.emit_by_name::<()>("activate", &[&0u32]);
}

fn find_list_view(widget: &gtk::Widget) -> Option<gtk::ListView> {
    if let Some(view) = widget.downcast_ref::<gtk::ListView>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        if let Some(found) = find_list_view(&node) {
            return Some(found);
        }
        child = node.next_sibling();
    }
    None
}
