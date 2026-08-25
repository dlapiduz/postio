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
//! point at [`postio_imap::test_server::TestServer`] on an ephemeral loopback
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
use postio_app::{commands, feed_the_window, start_syncing};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_imap::test_server::{TestMailbox, TestMessage, TestServer};
use postio_model::TransportSecurity;
use postio_session::{Wiring, actions};
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, test_support};

/// The corpus messages the server starts with, and the list must show.
const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

const INBOX_PATH: &str = "INBOX";

#[test]
fn a_keystroke_reaches_the_server_and_a_delivery_reaches_the_list() {
    let state_dir = std::env::temp_dir().join(format!("postio-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

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
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

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

    // ── 2. key → wire: `s` ends with the flag on the server's copy ────────
    //
    // The flag verb, not archive, and the choice is load-bearing: this
    // suite's first run found that a local move nulls the uid the drainer
    // later reads, so every archive is classified "never uploaded" and
    // silently skipped — #289. Flag writes keep their uid and drain
    // correctly, so they prove the key→wire direction today; #289's
    // definition of done includes flipping this phase to `a` and asserting
    // the message lands in the server's Archive.
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
        !server
            .flags(INBOX_PATH, uid)
            .contains(&postio_model::Flag::Flagged),
        "the fixture arrives unflagged, or flagging it proves nothing"
    );

    window.handle_key(
        gdk::Key::from_name("s").unwrap(),
        gdk::ModifierType::empty(),
    );

    // Local-first means the star appears immediately; the *server's* copy
    // changing is the queue draining over the wire, which is what no other
    // test can see.
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline
        && !server
            .flags(INBOX_PATH, uid)
            .contains(&postio_model::Flag::Flagged)
    {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(20));
    }
    if !server
        .flags(INBOX_PATH, uid)
        .contains(&postio_model::Flag::Flagged)
    {
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
            "the flag never landed on the server: queue [{states}], server flags for uid {uid:?}: {:?}, last commands={tail:?}",
            server.flags(INBOX_PATH, uid),
        );
    }

    // ── 3. server → window: a delivery mid-watch grows the list ───────────
    let shown_before = list.model().n_items();
    let commands_before = server.commands().len();
    server.deliver(INBOX_PATH, TestMessage::corpus("list-thread-01-root"));
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline && list.model().n_items() != shown_before + 1 {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(20));
    }
    if list.model().n_items() != shown_before + 1 {
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
            "the delivery never reached the list: showing {} of {shown_before}+1 rows, store holds {local} messages, server commands after deliver: {after:?}",
            list.model().n_items(),
        );
    }

    // The engine and window run until the process ends, like the binary; the
    // server's runtime must not block teardown on its live sessions.
    server_runtime.shutdown_background();
}
