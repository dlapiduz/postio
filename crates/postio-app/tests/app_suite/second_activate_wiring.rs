//! #514: a second `activate` must not re-wire the window over the first.
//!
//! `postio_gtk::app` builds the window and delivers a `gtk::Application`'s
//! `activate` signal; a single-instance app gets a second one whenever a
//! second launch just means "raise the window" — but `postio_app::run`'s own
//! handler called [`postio_app::open_or_onboard`] every time, unconditionally.
//! Nothing downstream of it was idempotent: a second `start_syncing` would
//! run a second set of engines against the store the first already opened,
//! and `commands::install`'s own `window.connect_action` stacks rather than
//! replaces, so a second install answers one keypress with two calls into
//! the bus.
//!
//! `open_or_onboard` cannot be driven through `activate` itself in a test —
//! that needs a real `gtk::Application` lifecycle — so this calls it twice
//! directly with the same `fed` cell, the way `startup_repair.rs` already
//! drives it once. The signal a doubled `connect_action` leaves is a flag
//! toggle: `Command::Flag` with no explicit state is a *toggle*, so if two
//! independent listeners both ran it for one keypress, the second call would
//! flip it straight back to where it started. A loopback `TestServer` is
//! what lets `start_syncing` run at all without touching a real network —
//! `attach_account.rs` established the same pattern for the same reason.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_app::notifications;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_imap::test_server::{TestMailbox, TestServer};
use postio_model::{Account, EmailAddress, Flag, Message, TransportSecurity};
use postio_session::{Wiring, actions};
use postio_storage::repository::{AccountRepository, MessageRepository};
use postio_storage::{BlobStore, test_support};
use std::sync::Arc;

const ADDRESS: &str = "ada@example.com";
const PASSWORD: &str = "hunter2";

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        settle();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

pub fn a_second_activate_does_not_double_wire_the_window() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-second-activate-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── a loopback server: the only way `start_syncing` may dial anything ──
    let server_runtime = tokio::runtime::Runtime::new().expect("a server runtime");
    let server = server_runtime.block_on(
        TestServer::builder()
            .account(ADDRESS)
            .password(PASSWORD)
            .mailbox(TestMailbox::new("INBOX"))
            .start(),
    );

    let database = test_support::memory();
    let (account_id, mailbox_id, message_id) = {
        let connection = database.connection().expect("a connection");
        let mut account = Account::new("Ada", EmailAddress::new(None::<String>, ADDRESS));
        account.incoming.host = server.addr().ip().to_string();
        account.incoming.port = server.addr().port();
        account.incoming.security = TransportSecurity::None;
        account.incoming.username = server.account().to_owned();
        let account_id = AccountRepository::new(&connection)
            .create(&mut account)
            .expect("the account row");

        let mailbox = test_support::mailbox(&connection, &account, "INBOX");

        // A message already local, so flagging it does not have to wait on
        // whatever the sync engine gets around to over the wire -- this test
        // is about the local command wiring, not about sync.
        let mut message = Message::new(account_id, mailbox.id, chrono::Utc::now());
        let message_id = MessageRepository::new(&connection)
            .create(&mut message)
            .expect("insert a message");
        (account_id, mailbox.id, message_id)
    };

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    server_runtime
        .block_on(secrets.store(&AccountKey::new(ADDRESS), &Password::new(PASSWORD)))
        .expect("the memory store accepts a password");

    // The real bus, over the real store -- the same composition `run()` uses,
    // so a doubled `connect_action` is the same bug it would be in the app.
    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<postio_core::CommandId> = bus.wired().collect();
    assert!(
        wired.contains(&postio_core::CommandId::Flag),
        "the bus does not answer flag, so this test cannot mean anything"
    );

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    )
    .with_secrets(secrets);

    let window = Window::default();
    window.present();
    settle();

    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    let events: Rc<RefCell<Option<_>>> = Rc::new(RefCell::new(Some(events)));
    let fed = Rc::new(Cell::new(false));

    // ── the same call twice, the same `fed` cell both times ────────────────
    // A second launch of a single-instance application delivers a second
    // `activate` to the primary process, and `activate`'s own handler makes
    // exactly this call every time -- see `postio_app::open_or_onboard`'s
    // doc comment for why `fed` is what stops the second one doing anything.
    for _ in 0..2 {
        postio_app::open_or_onboard(
            &window,
            &wiring,
            state.clone(),
            wired.clone(),
            Rc::clone(&events),
            notifier.clone(),
            Rc::clone(&fed),
        );
    }

    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "no rows arrived to press a key on"
    );

    // ── select the seeded message and flag it once ──────────────────────────
    let list = window.list();
    list.select_message(message_id);
    settle_until(|| list.cursor_id() == Some(message_id));
    settle();

    window.handle_key(
        gdk::Key::from_name("s").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();

    let flagged = || -> bool {
        let connection = database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .get(message_id)
            .expect("a read")
            .expect("still there")
            .flags
            .contains(&Flag::Flagged)
    };
    assert!(
        settle_until(flagged),
        "pressing `s` once should flag the message. If it did not, two \
         independent `connect_action` listeners (one per `open_or_onboard` \
         call) each toggled it -- true, then straight back to false -- which \
         is exactly what a second, unguarded `activate` used to cause"
    );

    let _ = (account_id, mailbox_id);

    bridge.shutdown();
}
