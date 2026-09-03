//! End to end: a window, the engine, and a real IMAP server in one process.
//!
//! Every other integration test stops at one side of the network joint —
//! deliberately. `keystroke.rs` proves a keypress reaches SQLite and never
//! opens a socket; `postio-sync/tests/loopback.rs` proves the engine's
//! primitives against real wire bytes and never builds a window. Both halves
//! pass while nothing joins them, and "layers pass while nothing joins them"
//! is this repository's most expensive recurring bug (`postio-bl2`, eight
//! instances). This file is that lesson applied to the last unjoined seam.
//!
//! It starts the way the binary starts: an account row whose server settings
//! point at [`postio_account::test_server::TestServer`] on an ephemeral loopback
//! port, a password in a [`MemorySecretStore`], a real [`Window`], and then
//! [`postio_app::start_syncing`] — the production path, which builds the real
//! connector, the real `io-imap` pool, and the real engine from that account
//! row. Three assertions, one per direction:
//!
//!   1. **wire → window** — the first sync's rows appear in the list.
//!   2. **key → wire** — `s` on a row ends with `\Flagged` on the *server's*
//!      copy, by the server's own flag accounting. (Archive is the sharper
//!      verb and is exactly what this suite's first run proved broken — the
//!      drainer never ships a local move, #289 — so the flag write carries
//!      this direction until that fix flips the phase to `a`.)
//!   3. **server → window** — a message delivered mid-watch grows the list.
//!
//! Two real bugs fell out of writing it — the sidebar's virtual-row sentinel
//! hijacking first-run folder selection, and #289 — which is the argument
//! for the suite in one sentence: every layer below had passing tests.
//!
//! Loopback only: `TransportSecurity::None` is refused for any non-loopback
//! host by `ConnectionSettings::validate`, so this test cannot be bent into
//! talking to a real network. Waits are event-polled with liveness-only
//! deadlines, per the under-load doctrine in `docs/engineering-notes.md`.
//!
//! One test function: GTK is single-threaded and initialised once per process.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. It runs as the first statement of a
// single-threaded test, which is the one moment it is sound. The crate's
// library code forbids `unsafe`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{TestMailbox, TestMessage, TestServer};
use postio_app::{commands, feed_the_window, start_syncing};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::TransportSecurity;
use postio_session::{Wiring, actions};
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, test_support};

/// The local row for `rfc_message_id`, if the store has one.
///
/// Phase 3 identifies the message it delivered rather than counting rows —
/// see there for why.
fn id_of(
    database: &postio_storage::Database,
    rfc_message_id: &str,
) -> Option<postio_model::MessageId> {
    let connection = database.connection().ok()?;
    connection
        .query_row(
            "SELECT id FROM messages WHERE rfc_message_id = ?1 AND deleted_locally = 0",
            [rfc_message_id],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .map(postio_model::MessageId::new)
}

/// The corpus messages the server starts with, and the list must show.
const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

/// The `Message-ID` of the fixture phase 3 delivers, so the row it produces
/// can be named rather than counted.
const DELIVERED_MESSAGE_ID: &str = "<harbour-dev.20260302T081200.a1@lists.example.org>";

const INBOX_PATH: &str = "INBOX";
const ARCHIVE_PATH: &str = "Archive";

#[test]
fn a_keystroke_reaches_the_server_and_a_delivery_reaches_the_list() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    // Off unless asked for. There was no way to see inside this test at all,
    // and diagnosing #364 meant adding one — which is a thing the next person
    // should not have to do again. `POSTIO_LOG=postio_runtime=debug` and so on.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            std::env::var("POSTIO_LOG").unwrap_or_else(|_| "off".into()),
        ))
        .with_test_writer()
        .try_init();
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── the server: real wire bytes on an ephemeral loopback port ─────────
    //
    // Its own runtime, kept for the life of the test: the server's accept
    // loop and sessions live on it, while the engine brings a runtime of its
    // own — exactly as the app and a real server own their halves.
    let server_runtime = tokio::runtime::Runtime::new().expect("a server runtime");
    let server = server_runtime.block_on(
        TestServer::builder()
            .account("test@example.com")
            .password("hunter2")
            .mailbox(TestMailbox::new("INBOX").corpus(SEEDED))
            .mailbox(TestMailbox::new("Archive").attributes(["\\Archive"]))
            .start(),
    );

    // ── the store: empty except the account row pointing at that server ───
    //
    // Nothing else is seeded. Every mailbox and message the window will show
    // has to arrive over the wire, which is the point.
    let database = test_support::memory();
    {
        let connection = database.connection().expect("a connection");
        let mut account = test_support::account(&connection);
        account.incoming.host = server.addr().ip().to_string();
        account.incoming.port = server.addr().port();
        account.incoming.security = TransportSecurity::None;
        account.incoming.username = server.account().to_owned();
        AccountRepository::new(&connection)
            .update(&mut account)
            .expect("the account row points at the test server");
    }
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let key = AccountKey::new("test@example.com");
    server_runtime
        .block_on(secrets.store(&key, &Password::new("hunter2")))
        .expect("the memory store accepts a password");

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // ── the application, assembled the way `run` assembles it ─────────────
    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    let (bridge, replies) = Bridge::new(bus).expect("a runtime");
    let (sink, engine_events) = event_channel();
    let mut wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );
    // The one substitution: this process has no Secret Service session, so
    // the password lives in the memory store the seam exists for.
    wiring.secrets = secrets;

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the store has an account")
        .feeds;
    commands::install(&window, &feeds, state, wiring.commands.clone(), wired);
    // Both event queues drain into the window, exactly as `open_account`
    // drains them — without this the engine can sync the world and the list
    // never hears about it, which is itself a postio-bl2-shaped wiring hole
    // this test exists to keep closed.
    let notifier = postio_app::notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    commands::drain(&window, &feeds, engine_events, notifier.clone());
    commands::drain(&window, &feeds, replies, notifier);

    // The production entry: reads the account row, builds the real connector
    // and pool, spawns the engine, starts the watch.
    start_syncing(&window, &wiring);

    // ── 1. wire → window: the first sync fills the list ───────────────────
    let list = window.list();
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline && list.model().n_items() != SEEDED.len() as u32 {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(20));
    }
    if list.model().n_items() != SEEDED.len() as u32 {
        let connection = database.connection().expect("a connection");
        let messages: i64 = connection
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap_or(-1);
        let mailboxes: i64 = connection
            .query_row("SELECT count(*) FROM mailboxes", [], |r| r.get(0))
            .unwrap_or(-1);
        panic!(
            "first sync never reached the list: server saw {} commands (first: {:?}), store holds {mailboxes} mailboxes / {messages} messages, list shows {} rows; sidebar default_mailbox={:?} selected={:?} list feed mailbox={:?}",
            server.commands().len(),
            server.commands().first(),
            list.model().n_items(),
            feeds.folders.default_mailbox(),
            window.sidebar().selected(),
            feeds.messages.mailbox(),
        );
    }

    // ── 2. key → wire: `a` ends with the message in the server's Archive ──
    //
    // The archive verb, deliberately: this suite's first run proved a local
    // move nulls the uid the drainer later reads, so every archive was
    // classified "never uploaded" and silently skipped — #289. The queue row
    // now snapshots the server coordinates at enqueue, and this phase is the
    // proof the whole path holds over a real wire: key press, local-first
    // move, drain, MOVE on the server.
    window.handle_key(
        gdk::Key::from_name("j").unwrap(),
        gdk::ModifierType::empty(),
    );
    while glib::MainContext::default().iteration(false) {}
    let focused = list
        .cursor_id()
        .expect("`j` should put the cursor on a synced row");
    let uid = {
        let connection = database.connection().expect("a connection");
        postio_storage::repository::MessageRepository::new(&connection)
            .get(focused)
            .expect("a read")
            .expect("the cursor row is in the store")
            .server
            .uid
            .expect("a synced row carries its server uid")
    };
    assert!(
        server.uids(ARCHIVE_PATH).is_empty(),
        "the fixture's Archive starts empty, or archiving proves nothing"
    );

    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );

    // Local-first means the row leaves the list immediately; the *server's*
    // copy moving is the queue draining over the wire, which is what no
    // other test can see.
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline && server.uids(ARCHIVE_PATH).is_empty() {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(20));
    }
    if server.uids(ARCHIVE_PATH).is_empty() {
        let connection = database.connection().expect("a connection");
        let states: String = connection
            .prepare("SELECT op_type, state, coalesce(last_error,'-') FROM operation_queue")
            .and_then(|mut st| {
                st.query_map([], |r| {
                    Ok(format!(
                        "{}:{}:{}",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?
                    ))
                })
                .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>().join(", "))
            })
            .unwrap_or_else(|e| format!("? ({e})"));
        let tail: Vec<String> = server.commands().into_iter().rev().take(6).collect();
        panic!(
            "the archive never landed on the server: queue [{states}], server INBOX={:?} Archive={:?}, last commands={tail:?}",
            server.uids(INBOX_PATH),
            server.uids(ARCHIVE_PATH),
        );
    }
    assert!(
        !server.uids(INBOX_PATH).contains(&uid),
        "the message reached Archive but was never taken out of INBOX: {:?}",
        server.uids(INBOX_PATH)
    );

    // ── 3. server → window: a delivery mid-watch reaches the list ─────────
    //
    // # Why this names the message instead of counting rows
    //
    // It used to assert `n_items() == shown_before + 1`, and that made the
    // suite fail about one run in eight (#364) — every time looking like a
    // regression in whatever was being landed, because it is the last gate
    // before a merge.
    //
    // The count was never the claim. `shown_before` is snapshotted straight
    // after phase 2, which waits for the message to reach the server's
    // Archive and *not* for anything local; the archived row's departure from
    // INBOX arrives separately and can still be outstanding here. Worse, it
    // can arrive and then be undone: an INBOX resync that runs before the
    // MOVE has drained re-creates the row it just removed, and a later resync
    // removes it again. So `shown_before` was 3 on some runs and 2 on others,
    // for the same correct behaviour, and only one of those makes
    // `shown_before + 1` reachable.
    //
    // Naming the message sidesteps all of it. "A delivery reaches the list"
    // is a statement about one message being on screen, so that is what is
    // asserted — of the row the server actually delivered, found by its
    // `Message-ID`, in the model the list is drawing from. It cannot pass for
    // the wrong reason the way a total can, and it does not care what the
    // archive is doing in the background.
    let commands_before = server.commands().len();
    assert!(
        id_of(&database, DELIVERED_MESSAGE_ID).is_none(),
        "the fixture phase 3 delivers is already in the store, so its arrival \
         would prove nothing"
    );
    server.deliver(INBOX_PATH, TestMessage::corpus("list-thread-01-root"));

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut delivered = None;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        delivered = id_of(&database, DELIVERED_MESSAGE_ID);
        if delivered.is_some_and(|id| list.model().position_of(id).is_some()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !delivered.is_some_and(|id| list.model().position_of(id).is_some()) {
        let connection = database.connection().expect("a connection");
        let local: i64 = connection
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap_or(-1);
        let after: Vec<String> = server
            .commands()
            .into_iter()
            .skip(commands_before)
            .collect();
        panic!(
            "the delivery never reached the list: the store {}, the list shows \
             {} rows and does not hold that message; store holds {local} \
             messages, server commands after deliver: {after:?}",
            match delivered {
                Some(id) => format!("has it as {id:?}"),
                None => "never got a row for it".to_owned(),
            },
            list.model().n_items(),
        );
    }

    // The engine and window run until the end, like the binary — and like the
    // binary, the engine is stopped before the process exits rather than left
    // writing into libraries that are being torn down. `postio_app::run` calls
    // this in the same place, right after the GTK loop returns.
    postio_runtime::stop_retained();

    // The server's runtime must not block teardown on its live sessions.
    server_runtime.shutdown_background();

    // The window this test built joins GTK's toplevel list at
    // construction and stays there, holding a WebProcess, until it is
    // destroyed -- which at exit() is a segfault after a passing test
    // (#794). No harness here to sweep, so the test does it.
    postio_gtk::window::close_all_windows();
}
