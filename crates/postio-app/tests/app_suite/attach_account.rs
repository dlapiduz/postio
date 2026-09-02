//! Issue #64: an account that joins an application which is already running.
//!
//! `e2e.rs` proves the *startup* path over a loopback server: an account row
//! that was there before the window existed ends up syncing. This proves the
//! half ADR 0012 Q2 is about -- that there is no longer anything
//! startup-specific about it. The window is built and fed first, over a
//! different account, and only then does a second account appear in the
//! store and get handed to [`postio_app::attach_account`]. Nothing is
//! restarted.
//!
//! Two things have to be true afterwards, and they fail independently:
//!
//!   1. **It syncs.** An engine of its own comes up and the mailboxes and
//!      messages it finds on the server land in the local store. Without
//!      this, add-account writes a row that does nothing until the next
//!      launch, which is the shape `settings_accounts.rs` already had to
//!      apologise for in its own module docs.
//!   2. **The surfaces know.** The settings panel lists it without being
//!      opened and closed again.
//!
//! Loopback only, like `e2e.rs`: `TransportSecurity::None` is refused for
//! any non-loopback host by `ConnectionSettings::validate`, so this cannot
//! be bent into talking to a real network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::gdk;
use postio_app::{attach_account, feed_the_window};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_imap::test_server::{TestMailbox, TestServer};
use postio_model::{Account, EmailAddress, TransportSecurity};
use postio_session::Wiring;
use postio_storage::repository::{AccountRepository, MailboxRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

/// What the joining account's server holds. One message is enough: the claim
/// is that its engine ran at all, not that sync is correct -- which is what
/// `postio-sync`'s own suites are for.
const SEEDED: [&str; 1] = ["plain-text-simple"];

const JOINING_ADDRESS: &str = "grace@example.com";
const JOINING_PASSWORD: &str = "hunter2";

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first -- copied from `settings_accounts_wiring.rs`
/// rather than shared, matching that file's own reason for copying it: no
/// dependency between the two.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if class.is_empty() || widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}

/// One widget per account the settings panel is showing.
fn account_rows(window: &Window) -> Vec<gtk::Widget> {
    collect(
        window.settings().upcast_ref::<gtk::Widget>(),
        "postio-settings-account-row",
    )
}

/// What each row says, so an assertion can name an address rather than an
/// index into a list whose order is the store's business.
fn labels_of(rows: Vec<gtk::Widget>) -> Vec<String> {
    rows.iter()
        .flat_map(|row| collect(row, ""))
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .map(|label| label.text().to_string())
        .collect()
}

pub fn an_account_added_to_a_running_application_syncs_without_a_restart() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: the suite runs its cases in sequence on one thread.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── the server the joining account will be pointed at ────────────────
    //
    // Its own runtime, kept for the life of the case: the accept loop and
    // the sessions live on it, while the engine brings a runtime of its own.
    let server_runtime = tokio::runtime::Runtime::new().expect("a server runtime");
    let server = server_runtime.block_on(
        TestServer::builder()
            .account(JOINING_ADDRESS)
            .password(JOINING_PASSWORD)
            .mailbox(TestMailbox::new("INBOX").corpus(SEEDED))
            .start(),
    );

    // ── an application already running over somebody else's mail ─────────
    let database = test_support::memory();
    let report = seed_small(&database, 51);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    )
    .with_secrets(secrets.clone());

    let window = Window::default();
    window.present();
    settle();
    feed_the_window(&window, &wiring).expect("the seeded store has an account");

    // The seeded account's own engine is deliberately never started: this
    // case is about the one that joins, and `start_syncing` would dial
    // `imap.example.com`, which no test in the default suite may do.
    assert_eq!(
        account_rows(&window).len(),
        1,
        "the application starts knowing about exactly the account it opened"
    );

    // ── the account appears in the store, the way a submission writes it ─
    let joining = {
        let connection = database.connection().expect("a connection");
        let mut account = Account::new("Grace", EmailAddress::new(None::<String>, JOINING_ADDRESS));
        account.incoming.host = server.addr().ip().to_string();
        account.incoming.port = server.addr().port();
        account.incoming.security = TransportSecurity::None;
        account.incoming.username = server.account().to_owned();
        AccountRepository::new(&connection)
            .create(&mut account)
            .expect("the joining account's row");
        account
    };
    server_runtime
        .block_on(secrets.store(
            &AccountKey::new(JOINING_ADDRESS),
            &Password::new(JOINING_PASSWORD),
        ))
        .expect("the memory store accepts a password");

    // ── the whole of what "join a running application" means ─────────────
    attach_account(&window, &wiring, &joining).expect("the pool can carry a second engine");

    // 1. it syncs: the folders and the mail arrive over the wire.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut synced = 0;
    while Instant::now() < deadline {
        settle();
        let connection = database.connection().expect("a connection");
        synced = MailboxRepository::new(&connection)
            .list_for_account(joining.id)
            .expect("a read")
            .len();
        drop(connection);
        if synced > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        synced > 0,
        "the joining account never synced: no engine came up for it, so \
         adding an account only takes effect at the next launch. The server \
         saw {} commands (first: {:?})",
        server.commands().len(),
        server.commands().first(),
    );

    // 2. the surfaces know: the settings panel lists it without being
    //    reopened.
    let listed = labels_of(account_rows(&window));
    assert!(
        listed.iter().any(|label| label.contains(JOINING_ADDRESS)),
        "the settings panel does not list the account that just joined \
         ({listed:?}): attaching started an engine and told no surface \
         about it"
    );
    // The account already there is not disturbed by the arrival of another.
    assert!(
        listed
            .iter()
            .any(|label| label.contains(report.account.address.address.as_str())),
        "attaching an account dropped the one that was already open \
         ({listed:?})"
    );

    bridge.shutdown();
}
