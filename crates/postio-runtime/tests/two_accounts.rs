//! Two engines, two accounts, one database — ADR 0005 Q3's concurrency test.
//!
//! One engine per account, each on its own thread with its own connection, is
//! the design `engine.rs` already builds; nothing about it assumed it was
//! alone. What this proves is the property multi-account exists for and the
//! one a global sync lock would destroy: **one slow or unreachable server
//! must not stall another account's mail.**
//!
//! The proof is by wall clock, not by assertion order: the slow account's
//! server answers every call with a deliberate latency, both passes start
//! together, and the fast account's pass has to *finish while the slow one is
//! still running*. WAL plus per-account operation queues are what make the
//! interleaved writes safe; each mailbox's `sync_state` row then has to
//! reflect only its own pass, because cross-talk there is silent corruption
//! rather than a crash.
//!
//! Nothing here touches the network: both servers are `MockBackend`s.

use std::sync::Arc;
use std::time::Duration;

use postio_core::bridge::event_channel;
use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
use postio_model::ids::MailboxId;
use postio_model::{Account, EmailAddress};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource};
use postio_storage::repository::{
    AccountRepository, MailboxRepository, MessageRepository, SyncStateRepository,
};
use postio_storage::{BlobStore, test_support};

/// Long enough that the fast pass finishing under it is a real ordering
/// proof on a loaded CI box, short enough that the suite stays quick.
const SLOW_LATENCY: Duration = Duration::from_millis(400);

/// One account with an INBOX, distinct from any other this test makes.
fn seeded_account(
    database: &postio_storage::Database,
    name: &str,
    address: &str,
) -> (postio_model::ids::AccountId, MailboxId) {
    let connection = database.connection().expect("a connection");
    let mut account = Account::new(name, EmailAddress::new(Some(name), address));
    account.incoming.host = "imap.example.com".to_owned();
    account.outgoing.host = "smtp.example.com".to_owned();
    AccountRepository::new(&connection)
        .create(&mut account)
        .expect("an account");
    let mut inbox = postio_model::Mailbox::new(account.id, "INBOX", Some('/'));
    MailboxRepository::new(&connection)
        .create(&mut inbox)
        .expect("an inbox");
    (account.id, inbox.id)
}

/// A server whose INBOX holds `count` messages, tagged so the account they
/// land in is provable from the subject.
fn server(tag: &str, count: u32) -> MockBackend {
    let mut inbox = MockMailbox::new("INBOX");
    for n in 1..=count {
        inbox = inbox.message(MockMessage::new(
            format!(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 To: Postio <postio@example.net>\r\n\
                 Subject: {tag} {n}\r\n\
                 Message-ID: <{tag}-{n}@example.com>\r\n\
                 Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
                 \r\n\
                 Mail for the {tag} account.\r\n"
            )
            .into_bytes(),
        ));
    }
    MockBackend::builder().mailbox(inbox).build()
}

fn spawn(
    database: &postio_storage::Database,
    account: postio_model::ids::AccountId,
    backend: Arc<MockBackend>,
) -> Engine {
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");
    let (sink, _events) = event_channel();
    Engine::spawn(EngineParts {
        account,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
    })
    .expect("the engine starts")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_account_does_not_stall_a_fast_one() {
    let database = test_support::memory();
    let (fast_account, fast_inbox) = seeded_account(&database, "Fast", "fast@example.com");
    let (slow_account, slow_inbox) = seeded_account(&database, "Slow", "slow@example.net");

    let fast_server = Arc::new(server("fast", 5));
    let slow_server = Arc::new(server("slow", 5));
    slow_server.set_latency(SLOW_LATENCY);

    let fast = spawn(&database, fast_account, fast_server);
    let slow = spawn(&database, slow_account, slow_server);

    // Both passes in flight together, exactly as N engines run them.
    let slow_pass = {
        let slow = slow.clone();
        tokio::spawn(async move { slow.sync(slow_inbox).await })
    };
    // Give the slow pass a head start into its first latency window, so
    // "the fast pass finished first" cannot be an artifact of spawn order.
    tokio::time::sleep(Duration::from_millis(20)).await;

    let started = std::time::Instant::now();
    fast.sync(fast_inbox).await.expect("the fast pass succeeds");
    let fast_took = started.elapsed();

    assert!(
        !slow_pass.is_finished(),
        "the slow pass was already done when the fast one finished, so this \
         run proves nothing about interference -- raise SLOW_LATENCY"
    );
    assert!(
        fast_took < SLOW_LATENCY,
        "the fast account waited out the slow account's latency ({fast_took:?}), \
         which is exactly the serialisation a global sync lock would cause"
    );

    slow_pass
        .await
        .expect("the slow task did not panic")
        .expect("the slow pass succeeds");

    // -- Each account got its own mail, and only its own -------------------
    let connection = database.connection().expect("a connection");
    let messages = MessageRepository::new(&connection);
    for (account, inbox, tag) in [
        (fast_account, fast_inbox, "fast"),
        (slow_account, slow_inbox, "slow"),
    ] {
        // Scoped to the *mailbox*, so a message that landed in the wrong
        // account's inbox would be missing here and counted twice there.
        let page = messages
            .page(
                &postio_storage::repository::ListQuery {
                    scope: postio_storage::repository::ListScope::Mailbox(inbox),
                    ..postio_storage::repository::ListQuery::account(account)
                }
                .limit(50),
            )
            .expect("a page");
        assert_eq!(page.len(), 5, "{tag}: every message landed");
        for row in &page {
            let subject = row.subject.as_deref().unwrap_or_default();
            assert!(
                subject.starts_with(tag),
                "{tag}: a message from the other account's server crossed \
                 over: {subject:?}"
            );
        }
    }

    // -- And each sync_state row reflects only its own pass ---------------
    let states = SyncStateRepository::new(&connection);
    for (inbox, tag) in [(fast_inbox, "fast"), (slow_inbox, "slow")] {
        let state = states
            .get(inbox)
            .expect("a read")
            .unwrap_or_else(|| panic!("{tag}: no sync_state row after a completed pass"));
        assert!(
            state.last_full_sync_at.is_some(),
            "{tag}: the pass completed but its own row does not say so"
        );
    }
}
