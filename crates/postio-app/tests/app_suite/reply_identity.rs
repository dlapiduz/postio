//! Issue #189 (ADR 0005 Q13): a reply must resolve the identity of the
//! account the message *arrived at*, never whatever account the window
//! opened on. Getting this wrong sends from the wrong address and is
//! discovered only after sending.
//!
//! `install_reply_source` (`crates/postio-app/src/compose.rs`) already reads
//! `message.account_id` off the message the reading pane is showing, not off
//! the account `feed_the_window` picked as the window's own default — but
//! nothing proved it at the layer #325 hid in: the composition root's real
//! wiring, not a composer test with a stubbed provider.
//! `gtk_composer_reply.rs` proves `reply_draft` takes whatever `Account` it
//! is handed; it cannot see whether the *right* one was ever handed to it.
//!
//! The message being replied to belongs to a mailbox `feed_the_window`'s
//! single-account wiring never fed the sidebar — `window.open_message` is
//! used to reach it anyway, the same entry point a notification click uses,
//! because the store underneath is one shared database keyed by mailbox id,
//! not partitioned per account the window happens to be showing.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, Identity, Message};
use postio_session::Wiring;
use postio_storage::repository::{AccountRepository, MessageRepository};
use postio_storage::{Database, test_support};

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

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

/// A configured, enabled account with one default identity at `address`.
fn account_with_identity(database: &Database, display_name: &str, address: &str) -> Account {
    let connection = database.connection().expect("a connection");
    let mut account = Account::new(display_name, EmailAddress::new(None::<String>, address));
    account.identities = vec![{
        let mut identity = Identity::new(
            AccountId::UNASSIGNED,
            EmailAddress::new(None::<String>, address),
        );
        identity.display_name = display_name.to_owned();
        identity.is_default = true;
        identity
    }];
    AccountRepository::new(&connection)
        .create(&mut account)
        .expect("create the account");
    account
}

pub fn a_reply_to_a_message_in_a_second_account_uses_that_accounts_identity() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();

    // Account A is created first, so `first_account` -- and therefore the
    // window's own default identity -- resolves to it, not to B.
    let account_a = account_with_identity(&database, "Personal", "ada@example.com");
    let account_b = account_with_identity(&database, "Work", "grace@example.com");
    assert_ne!(
        account_a.id, account_b.id,
        "the two accounts must be genuinely distinct"
    );

    let inbox_b = {
        let connection = database.connection().expect("a connection");
        test_support::mailbox(&connection, &account_b, "INBOX")
    };
    let message_b = {
        let connection = database.connection().expect("a connection");
        let mut message = Message::new(account_b.id, inbox_b.id, chrono::Utc::now());
        message.from = vec![EmailAddress::new(Some("Quinn Abara"), "quinn@example.com")];
        message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
        message.subject = Some("Quarterly numbers".to_owned());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create the message in account B")
    };

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();

    // The same call `run` makes -- account A is what this wires the window
    // to, exactly as it would with one account configured.
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");

    let composer = window.composer();

    // Reach the message in account B the way a notification click does --
    // `feed_the_window`'s sidebar only ever learned about account A's
    // mailboxes, and this is the entry point built for exactly that case.
    window.open_message(inbox_b.id, message_b);
    let list = window.list();
    assert!(
        settle_until(|| list.cursor_id() == Some(message_b)),
        "the cursor never reached the message in account B"
    );
    // Through the same activation `resume_draft.rs` and `reply_source.rs`
    // use in tests: a key never reaches the widget here, and this message
    // just became the cursor's row by autoselect rather than a real move, so
    // nothing has reported it to the reading pane yet.
    list.test_activate_cursor();
    assert!(
        settle_until(|| window.reading()),
        "the pane never showed the message in account B"
    );

    press(&window, "e");
    assert!(
        composer.is_open(),
        "`e` on a message in a second account did not open a reply"
    );
    let draft = composer.draft();
    assert_eq!(
        draft.account_id, account_b.id,
        "the reply used account A's identity for a message that arrived at \
         account B"
    );
    let identity = composer
        .identity()
        .expect("a reply to a message with a matching recipient resolves an identity");
    assert_eq!(
        identity.address, account_b.identities[0].address,
        "the resolved identity was not account B's own address"
    );
    composer.discard();
    settle();

    bridge.shutdown();
}
