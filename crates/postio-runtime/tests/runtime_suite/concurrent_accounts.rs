use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::bridge::event_channel;
use postio_model::AccountId;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{AccountRepository, MailboxRepository, SyncStateRepository};
use postio_storage::{BlobStore, Store, test_support};

use crate::harness;

use harness::BlobDir;

/// The assertion is on ordering, not elapsed time: an absolute wall-clock bound measured machine
/// load.
const SLOW: Duration = Duration::from_millis(400);

fn a_message() -> Vec<u8> {
    "From: Ada Lovelace <ada@example.com>\r\n\
     To: Postio <postio@example.net>\r\n\
     Subject: one\r\n\
     Message-ID: <one@example.com>\r\n\
     Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
     \r\n\
     The bytes that had to travel to get here.\r\n"
        .to_owned()
        .into_bytes()
}

fn server() -> MockBackend {
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(MockMessage::new(a_message())))
        .build()
}

fn engine_for(
    database: &Store,
    account: AccountId,
    backend: Arc<MockBackend>,
) -> (Engine, BlobDir) {
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("an engine per account");
    let directory = BlobDir::new(engine.clone(), directory);
    (engine, directory)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_account_does_not_hold_up_a_fast_one() {
    let database = test_support::memory().await;

    let (slow, fast) = {
        let connection = database.connect().await.expect("a connection");
        let accounts = AccountRepository::new(&connection);

        let mut slow_account = postio_model::Account::new(
            "Slow",
            postio_model::EmailAddress::new(Some("Slow"), "slow@example.com"),
        );
        let mut fast_account = postio_model::Account::new(
            "Fast",
            postio_model::EmailAddress::new(Some("Fast"), "fast@example.com"),
        );
        accounts
            .create(&mut slow_account)
            .await
            .expect("the slow account");
        accounts
            .create(&mut fast_account)
            .await
            .expect("the fast account");

        let slow_inbox = test_support::mailbox(&connection, &slow_account, "INBOX").await;
        let fast_inbox = test_support::mailbox(&connection, &fast_account, "INBOX").await;
        (
            (slow_account.id, slow_inbox.id),
            (fast_account.id, fast_inbox.id),
        )
    };

    let slow_backend = Arc::new(server());
    slow_backend.set_latency(SLOW);
    let fast_backend = Arc::new(server());

    let (slow_engine, _slow_directory) = engine_for(&database, slow.0, Arc::clone(&slow_backend));
    let (fast_engine, _fast_directory) = engine_for(&database, fast.0, Arc::clone(&fast_backend));

    let slow_pass = tokio::spawn(async move {
        let outcome = slow_engine.sync(slow.1).await;
        (outcome, Instant::now())
    });

    // Get the slow account genuinely in flight, or finishing first could mean it had not started.
    tokio::time::sleep(Duration::from_millis(50)).await;

    fast_engine
        .sync(fast.1)
        .await
        .expect("the fast account syncs");
    let fast_finished = Instant::now();

    let (slow_outcome, slow_finished) = slow_pass.await.expect("the slow task");
    slow_outcome.expect("the slow account syncs too, eventually");

    // Ordering, not duration: anything serializing the two makes the fast pass wait, and both
    // accounts are loaded equally.
    assert!(
        fast_finished < slow_finished,
        "the fast account did not finish until after the slow one — it waited \
         for it, so something is serializing the two"
    );

    let connection = database.connect().await.expect("a connection");
    let sync_state = SyncStateRepository::new(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    for (account, inbox) in [slow, fast] {
        for folder in mailboxes
            .list_for_account(account)
            .await
            .expect("its folders")
        {
            assert_eq!(
                folder.account_id, account,
                "a folder was filed under an account that does not own it"
            );
        }
        let state = sync_state
            .get(inbox)
            .await
            .expect("its sync state")
            .expect("a pass was recorded for this account's own inbox");
        assert_eq!(
            state.mailbox_id, inbox,
            "a sync_state row belongs to the mailbox its pass covered, not to \
             whichever account happened to finish last"
        );
    }
}
