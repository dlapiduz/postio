//! End to end: Focus's window, the engine, and a real IMAP server in one
//! process.
//!
//! Every other case stops at one side of the network joint -- deliberately.
//! This one is that joint, the way Focus starts: an account row whose server
//! settings point at [`TestServer`] on an ephemeral loopback port, its
//! password in a `MemorySecretStore`, a real `FocusWindow` adopted over a
//! host, and then `Session::start_syncing`, which builds the real connector,
//! the real `io-imap` pool and the real engine from that account row. Three
//! assertions, one per direction:
//!
//!   1. **wire -> window**: the first sync's rows appear in the list.
//!   2. **key -> wire**: `a` on a row ends with the message in the
//!      *server's* Archive and out of its INBOX.
//!   3. **server -> window**: a message delivered mid-watch is listed.
//!
//! Loopback only: `TransportSecurity::None` is refused for any non-loopback
//! host by `ConnectionSettings::validate`, so this cannot be bent into
//! talking to a real network. Waits are event-polled with liveness-only
//! deadlines.

use std::sync::Arc;

use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{TestMailbox, TestMessage, TestServer};
use postio_model::TransportSecurity;
use postio_storage::bind;
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, test_support};

use crate::support;

/// The corpus messages the server starts with.
const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

/// The `Message-ID` of the fixture phase 3 delivers, so the row it produces
/// can be named rather than counted.
const DELIVERED_MESSAGE_ID: &str = "<harbour-dev.20260302T081200.a1@lists.example.org>";

const INBOX_PATH: &str = "INBOX";
const ARCHIVE_PATH: &str = "Archive";

/// The local row for `rfc_message_id`, if the store has one, and its subject.
async fn local(
    database: &postio_storage::Store,
    rfc_message_id: &str,
) -> Option<(postio_model::MessageId, String)> {
    let connection = database.connect().await.ok()?;
    postio_storage::sql::one(
        &connection,
        "SELECT id, coalesce(subject, '') FROM messages \
         WHERE rfc_message_id = ?1 AND deleted_locally = 0",
        bind![rfc_message_id],
        |row| {
            Ok((
                postio_model::MessageId::new(postio_storage::sql::RowExt::col::<i64>(row, 0)?),
                postio_storage::sql::RowExt::col::<String>(row, 1)?,
            ))
        },
    )
    .await
    .ok()
}

pub fn a_keystroke_reaches_the_server_and_a_delivery_reaches_the_list() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        // The server: real wire bytes on an ephemeral loopback port.
        let server = TestServer::builder()
            .account("test@example.com")
            .password("hunter2")
            .mailbox(TestMailbox::new("INBOX").corpus(SEEDED))
            .mailbox(TestMailbox::new("Archive").attributes(["\\Archive"]))
            .start()
            .await;

        // The store: empty except the account row pointing at that server.
        // Everything the window shows has to arrive over the wire.
        let database = test_support::memory().await;
        {
            let connection = database.connect().await.expect("a connection");
            let mut account = test_support::account(&connection).await;
            account.incoming.host = server.addr().ip().to_string();
            account.incoming.port = server.addr().port();
            account.incoming.security = TransportSecurity::None;
            account.incoming.username = server.account().to_owned();
            AccountRepository::new(&connection)
                .update(&mut account)
                .await
                .expect("the account row points at the test server");
        }
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
        secrets
            .store(
                &AccountKey::new("test@example.com"),
                &Password::new("hunter2"),
            )
            .await
            .expect("the memory store accepts a password");
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
        // The one substitution: this process has no Secret Service session.
        let host = postio_host::Host::start(database.clone(), blobs, {
            let secrets = Arc::clone(&secrets);
            move |wiring| wiring.with_secrets(secrets)
        })
        .expect("a host");

        let window = postio_focus::window::FocusWindow::new(None);
        gtk::prelude::GtkWindowExt::present(&window);
        let session =
            postio_focus::startup::adopt(&window, host, &postio_config::Config::default());
        // What `app.rs` does once the stored mail is drawn: the production
        // entry, which reads the account row and starts the engine.
        session.start_syncing();

        // 1. wire -> window: the first sync fills the list.
        let listed =
            crate::settle_until(async || support::subjects(&window).iter().any(|s| !s.is_empty()))
                .await;
        assert!(
            listed,
            "first sync never reached the list: server saw {} commands (first: {:?}), \
             the list shows {:?}",
            server.commands().len(),
            server.commands().first(),
            support::subjects(&window)
        );
        // The store has all three; the list shows what Focus keeps in its
        // inbox of them.
        assert!(
            crate::settle_until(async || {
                let connection = database.connect().await.expect("a connection");
                postio_storage::sql::one(&connection, "SELECT count(*) FROM messages", (), |r| {
                    postio_storage::sql::RowExt::col::<i64>(r, 0)
                })
                .await
                .unwrap_or(0)
                    == SEEDED.len() as i64
            })
            .await,
            "the store never held the server's {} messages",
            SEEDED.len()
        );

        // 2. key -> wire: `a` ends with the message in the server's Archive.
        support::keys(&window, &["j"]);
        let focused = window
            .cursor_row()
            .and_then(|row| row.as_conversation().map(|c| c.summary.representative.id))
            .expect("`j` should put the cursor on a synced row");
        let uid = {
            let connection = database.connect().await.expect("a connection");
            postio_storage::repository::MessageRepository::new(&connection)
                .get(focused)
                .await
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
        support::keys(&window, &["a"]);
        // Local-first means the row leaves the list at once; the server's
        // copy moving is the queue draining over the wire.
        assert!(
            crate::settle_until(async || !server.uids(ARCHIVE_PATH).is_empty()).await,
            "the archive never landed on the server: INBOX={:?} Archive={:?}, last commands={:?}",
            server.uids(INBOX_PATH),
            server.uids(ARCHIVE_PATH),
            server
                .commands()
                .into_iter()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
        );
        assert!(
            !server.uids(INBOX_PATH).contains(&uid),
            "the message reached Archive but was never taken out of INBOX: {:?}",
            server.uids(INBOX_PATH)
        );

        // 3. server -> window: a delivery mid-watch reaches the list. It
        // names the message rather than counting rows: the archived row's
        // departure arrives separately, so a total moves for reasons that
        // are not this one's.
        assert!(
            local(&database, DELIVERED_MESSAGE_ID).await.is_none(),
            "the fixture phase 3 delivers is already in the store, so its \
             arrival would prove nothing"
        );
        server.deliver(INBOX_PATH, TestMessage::corpus("list-thread-01-root"));
        let subject: std::cell::RefCell<Option<String>> = Default::default();
        let arrived = crate::settle_until(async || {
            if subject.borrow().is_none() {
                let found = local(&database, DELIVERED_MESSAGE_ID).await;
                *subject.borrow_mut() = found.map(|(_, subject)| subject);
            }
            subject
                .borrow()
                .as_ref()
                .is_some_and(|subject| support::subjects(&window).contains(subject))
        })
        .await;
        assert!(
            arrived,
            "the delivery never reached the list: the store {}, the list shows {:?}",
            match &*subject.borrow() {
                Some(subject) => format!("has it as {subject:?}"),
                None => "never got a row for it".to_owned(),
            },
            support::subjects(&window)
        );

        session.stop();
        support::keep(session);
    });
}
