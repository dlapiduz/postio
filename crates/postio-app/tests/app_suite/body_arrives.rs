//! Issue #396: a body that lands has to reach the pane that is waiting for it.
//!
//! `postio_runtime::engine` emits [`Event::BodyLoaded`] the moment a fetched
//! body is committed, and until this test nothing in the workspace acted on
//! it. The event exists, is documented, and is covered by
//! `postio-core/tests/events.rs` — every layer passes, and a person who
//! clicked a message whose body was not local kept looking at the
//! "Downloading this message" plate until something else happened to force a
//! redraw. That is `postio-bl2`'s shape one layer up, which is why the
//! assertions here are about the *pane*, never about what a feed was handed.
//!
//! # Four things, one window
//!
//! The four acceptance criteria are one story and share one store, so they
//! are one test function (the reason `wiring.rs` gives):
//!
//! 1. an arrival for a message nobody is looking at repaints nothing — proved
//!    by writing the *shown* message's body first, so an indiscriminate
//!    repaint would visibly flip the pane and this must not;
//! 2. an arrival for the message in the pane repaints it;
//! 3. a payload arriving updates the chip in the parts panel, without moving
//!    the cursor out from under whoever is standing on it;
//! 4. a burst is coalesced into one repaint, the way `SidebarFeed::reload`
//!    coalesces a resync's `MessagesChanged`.
//!
//! # It does not dial anything
//!
//! `feed_the_window` reads the local store; `start_syncing` is the half that
//! opens a socket and this never calls it. An arrival is simulated the way
//! `reading_offline.rs` simulates connectivity — the store is written, then
//! `Feeds::apply` is handed the event a real engine would emit — which is
//! exactly the seam `commands::drain` feeds in the running application.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_core::{ConnectionState, Event};
use postio_gtk::reader::Absent;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::MessageId;
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{BlobStore, Database, test_support};

/// The message the pane will be showing: words plus one named attachment, so
/// the same message proves both the body arriving and the payload arriving.
const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Quarterly figures\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"edge\"\r\n\
\r\n\
--edge\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
See the attached figures.\r\n\
--edge\r\n\
Content-Type: text/csv\r\n\
Content-Disposition: attachment; filename=\"figures.csv\"\r\n\
\r\n\
one,two\r\n\
--edge--\r\n";

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// Give the application every chance to repaint, and answer whether it left
/// `held` true throughout.
///
/// The mirror of [`settle_until`], for the criterion that is about something
/// *not* happening: a repaint that should never have been queued cannot be
/// waited for, only ruled out.
fn settle_while(held: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if !held() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    held()
}

pub fn a_body_that_lands_repaints_the_pane_waiting_for_it_and_no_other() {
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

    // ── a store holding two messages, neither with a body ────────────────
    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (account, shown, other) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);

        // The older one, which the pane will never be showing.
        let mut other = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::hours(1),
        );
        other.subject = Some("Last week's figures".into());
        other.sync.body_state = BodyState::HeadersOnly;
        let other = repository.create(&mut other).expect("a message");

        // The newest one, which the list opens on.
        let parsed = postio_model::mime::parse(RAW);
        let mut shown = Message::new(account.id, inbox, chrono::Utc::now());
        shown.subject = Some("Quarterly figures".into());
        shown.sync.body_state = BodyState::HeadersOnly;
        shown.content_type = Some("multipart/mixed".into());
        shown.attachments = parsed
            .parts
            .iter()
            .map(|part| part.attachment.clone())
            .collect();
        let shown = repository.create(&mut shown).expect("a message");

        (account.id, shown, other)
    };

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

    // ── the same call `run` makes ────────────────────────────────────────
    let wired = feed_the_window(&window, &wiring).expect("the store has an account");

    let list = window.list();
    assert!(
        settle_until(|| list.model().n_items() > 0),
        "the list is empty, so there is nothing to open"
    );
    activate_first_row(&window);
    assert!(
        settle_until(|| window.reading()),
        "the message was opened and the reading pane never filled"
    );

    // Online, so the plate is the ordinary backfill wait rather than #117's
    // offline one — which is the state a body is actually awaited in.
    wired.feeds.apply(&Event::ConnectionChanged {
        account,
        state: ConnectionState::Online,
    });
    assert!(
        settle_until(|| window.reader().absent() == Some(Absent::Partial)),
        "the pane should be waiting on a body it has not got: got {:?}",
        window.reader().absent()
    );

    // ── 1. an arrival for a message nobody is looking at ────────────────
    //
    // The shown message's body is written *first*, so a repaint that did not
    // check who it was for would flip this pane to a rendered body. Only a
    // consumer that reads the event's `message` can leave the plate up.
    store_body(&database, shown, "See the attached figures.");
    wired.feeds.apply(&Event::BodyLoaded {
        account,
        message: other,
    });
    assert!(
        settle_while(|| window.reader().absent() == Some(Absent::Partial)),
        "a body arriving for a message the user is not looking at repainted \
         the pane anyway — the event carries which message it is for, and \
         the consumer has to read it"
    );

    // ── 2. and one for the message in the pane ──────────────────────────
    let before = window.reader().paints();
    wired.feeds.apply(&Event::BodyLoaded {
        account,
        message: shown,
    });
    assert!(
        settle_until(|| window.reader().absent().is_none()),
        "the body for the message in the pane landed and the pane went on \
         showing the wait. `Event::BodyLoaded` is emitted the moment the \
         bytes are committed; check that anything at all consumes it"
    );

    // ── 4. and it did so once, not once per event ───────────────────────
    //
    // Here rather than after a burst of its own, because the burst has to be
    // *the same* repaint this criterion is about. Twenty events in one turn
    // of the main loop is what a backfill looks like from the pane's side.
    let once = window.reader().paints();
    assert_eq!(
        once - before,
        1,
        "one arrival should be one repaint, not {}",
        once - before
    );
    for _ in 0..20 {
        wired.feeds.apply(&Event::BodyLoaded {
            account,
            message: shown,
        });
    }
    assert!(settle_while(|| window.reader().absent().is_none()));
    // Zero, not one (#749). Coalescing is still what stops twenty events
    // being twenty repaints — the assertion above proves the first arrival
    // was exactly one — but the body is on screen now, so these twenty
    // recompose the identical document and the pane must not be torn down
    // and rebuilt for them at all. Every rebuild is a frame of unpainted
    // WebView and a scroll position discarded, which is what a person reading
    // a long message actually notices.
    assert_eq!(
        window.reader().paints() - once,
        0,
        "twenty arrivals that change nothing on screen repainted the pane {} \
         time(s) — a repaint has to be worth its cost, not merely coalesced",
        window.reader().paints() - once
    );

    // ── 3. a payload arriving updates its chip ──────────────────────────
    let chip = settle_for_chip(&window).expect("the message has a named attachment");
    chip.emit_clicked();
    while glib::MainContext::default().iteration(false) {}
    assert!(
        window.parts().is_visible(),
        "the chip should open the panel"
    );

    let panel = window.parts();
    while panel.cursor().map(|node| node.mime) != Some("text/csv".to_owned()) {
        assert!(
            window.handle_key(gdk::Key::j, gdk::ModifierType::empty()) == glib::Propagation::Stop,
            "walked off the end of the tree before finding the attachment"
        );
    }
    assert!(
        !panel.cursor().expect("a cursor").downloaded,
        "nothing has been fetched for this part yet"
    );

    // The payload axis' commit point, as `backfill::fetch_payloads` writes it.
    {
        let connection = database.connection().expect("a connection");
        let blob = blobs.put(b"one,two").expect("a blob");
        MessageRepository::new(&connection)
            .set_attachment_blob(shown, "2", &blob)
            .expect("the part's bytes are recorded");
    }
    wired.feeds.apply(&Event::BodyLoaded {
        account,
        message: shown,
    });
    assert!(
        settle_until(|| panel.cursor().is_some_and(|node| node.downloaded)),
        "the bytes are on the disk and the chip still says \"download\" — \
         #377 made `Node::downloaded` change at runtime, so the panel has to \
         be told when it does"
    );
    assert_eq!(
        panel.cursor().map(|node| node.mime),
        Some("text/csv".to_owned()),
        "refreshing the panel moved the cursor out from under the person \
         standing on the part they are waiting for"
    );

    bridge.shutdown();
}

/// Write `text` as `message`'s body, as a completed fetch leaves it.
fn store_body(database: &Database, message: MessageId, text: &str) {
    let connection = database.connection().expect("a connection");
    MessageRepository::new(&connection)
        .set_body(
            message,
            &StoredBody {
                text: Some(text.to_owned()),
                html: None,
                headers: None,
            },
            BodyState::Full,
        )
        .expect("the body is stored");
}

/// The one chip a message with a named attachment gets, once it appears.
fn settle_for_chip(window: &Window) -> Option<gtk::Button> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if let Some(chip) = chips(window).into_iter().next() {
            return Some(chip);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

fn chips(window: &Window) -> Vec<gtk::Button> {
    let mut found = Vec::new();
    walk(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.has_css_class("postio-attachment")
        {
            found.push(button.clone());
        }
    });
    found
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(node) = child {
        walk(&node, visit);
        child = node.next_sibling();
    }
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
