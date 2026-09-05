//! The unsubscribe banner, wired end to end (#971): a click in the reader
//! reaches the store, and the privacy pane reads it back.
//!
//! `postio-gtk` has no SQL, so `Reader::connect_unsubscribe_activated` only
//! asks — this proves the composition root answers: the activation lands in
//! `UnsubscribeRepository`, stamped with the message's own account, and
//! opening settings afterwards lists it. The same two-part shape
//! `egress_wiring.rs` proves for the connection log, and the same
//! hand-authored single-message shape `decode_notice.rs` uses rather than
//! walking the corpus for a row with the property under test.
//!
//! One test function, for the reason `wiring.rs` gives.

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
use postio_model::BodyState;
use postio_storage::repository::{MessageRepository, StoredBody, UnsubscribeRepository};
use postio_storage::{BlobStore, Database, test_support};

/// A plain message with no `List-Id` — the sender's domain is #971's own
/// fallback, so this exercises that path without depending on `mail_parser`'s
/// handling of RFC 2919's list headers, which the model layer already tests.
const NEWSLETTER: &[u8] = b"From: Weekly Digest <weekly@news.example.org>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: This week in Postio\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Nothing much happened\r\n";

/// Store `raw` as a message with a body, the way the backfill commits one.
fn store(
    database: &Database,
    account: postio_model::ids::AccountId,
    mailbox: postio_model::ids::MailboxId,
    raw: &[u8],
) -> postio_model::ids::MessageId {
    let connection = database.connection().expect("a connection");
    let repository = MessageRepository::new(&connection);
    let parsed = postio_model::mime::parse(raw);
    let body = parsed.body.clone();
    let encoding_problems = parsed.encoding_problems;

    // `into_message`, not `Message::new` plus a couple of fields: the
    // banner needs `from` (#971's domain fallback), which only the parsed
    // headers carry.
    let mut message = parsed.into_message(account, mailbox, chrono::Utc::now());
    message.sync.body_state = BodyState::Full;
    let id = repository.create(&mut message).expect("a message");

    let stored = StoredBody {
        text: body.text,
        html: body.html,
        headers: None,
        headers_truncated: false,
        encoding_problems,
    };
    repository
        .set_body(id, &stored, BodyState::Full)
        .expect("a body");
    id
}

pub fn clicking_unsubscribe_logs_the_activation_and_the_privacy_pane_lists_it() {
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

    let account = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        drop(connection);
        store(&database, account.id, inbox, NEWSLETTER);
        account.id
    };

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
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
        settle_until(|| list.model().n_items() == 1),
        "the message never reached the list"
    );

    activate_first_row(&window);
    assert!(
        settle_until(|| window.reading()),
        "the message was opened and the reading pane never filled"
    );
    assert!(
        settle_until(|| window.reader().unsubscribe_banner_visible()),
        "a message with a sender should always have a list to leave -- the \
         domain fallback if nothing else"
    );

    // ── the store starts with no activation logged ───────────────────────
    {
        let connection = database.connection().expect("a connection");
        assert_eq!(
            UnsubscribeRepository::new(&connection)
                .for_account(account)
                .expect("list")
                .len(),
            0,
            "nothing has been activated yet"
        );
    }

    // ── clicking unsubscribe reaches the store, account stamped ──────────
    window.reader().click_unsubscribe();
    let landed = settle_until(|| {
        let connection = database.connection().expect("a connection");
        UnsubscribeRepository::new(&connection)
            .for_account(account)
            .expect("list")
            .len()
            == 1
    });
    assert!(
        landed,
        "the click never reached storage -- the reader only asks, and \
         nothing answered"
    );
    {
        let connection = database.connection().expect("a connection");
        let logged = UnsubscribeRepository::new(&connection)
            .for_account(account)
            .expect("list");
        assert_eq!(logged[0].account_id, account);
        assert_eq!(
            logged[0].list_identifier, "news.example.org",
            "no List-Id on this fixture, so the sender's domain is what \
             should have been logged"
        );
    }

    // ── opening settings lists it ────────────────────────────────────────
    window.act(postio_core::Command::Settings);
    while glib::MainContext::default().iteration(false) {}
    let listed = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-settings-unsubscribe-row")
    });
    assert!(
        listed.is_some(),
        "the privacy pane lists no activations over a log that holds one"
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

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
