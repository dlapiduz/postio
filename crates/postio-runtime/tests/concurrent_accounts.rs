//! Two accounts syncing at once, without one waiting on the other.
//!
//! ADR 0005 Q3, and #1's first acceptance criterion (#183). `Engine` was
//! already keyed to one account with its own connection and its own queue;
//! nothing in it assumed it was alone. What was missing was any proof of
//! that, and a composition root willing to start a second one.
//!
//! The shape ADR 0005 asks for: two `MailBackend` mocks with different
//! latencies, and the fast account's pass finishing while the slow account's
//! is still in flight. A test that merely started two engines and waited for
//! both would pass just as well against a global lock, which is the thing
//! this exists to rule out.
//!
//! Both mocks are in-memory and no SMTP connector is ever dialled. Nothing
//! here touches the network.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::bridge::event_channel;
use postio_model::AccountId;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{AccountRepository, MailboxRepository, SyncStateRepository};
use postio_storage::{BlobStore, Database, test_support};

mod harness;

use harness::BlobDir;

/// How long every call to the slow account's server takes.
///
/// The assertion below is on *ordering*, not on elapsed time, so this only has
/// to make the slow account reliably slower than the fast one — not to fit
/// inside any particular budget. An earlier version asserted the fast pass
/// finished within this bound and failed on the land gate at 2.2s: the whole
/// machine was busy compiling, and an absolute wall-clock bound measures the
/// load as much as the code. A test that fails when the box is busy is worse
/// than no test.
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

/// One folder holding one message, so a pass is short and the latency is what
/// dominates it.
fn server() -> MockBackend {
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(MockMessage::new(a_message())))
        .build()
}

fn engine_for(
    database: &Database,
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

/// The ADR's own test: a slow account must not hold up a fast one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_account_does_not_hold_up_a_fast_one() {
    let database = test_support::memory();

    let (slow, fast) = {
        let connection = database.connection().expect("a connection");
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
            .expect("the slow account");
        accounts
            .create(&mut fast_account)
            .expect("the fast account");

        let slow_inbox = test_support::mailbox(&connection, &slow_account, "INBOX");
        let fast_inbox = test_support::mailbox(&connection, &fast_account, "INBOX");
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

    // The slow pass starts first, so anything serializing the two would have
    // to finish it before the fast one could begin.
    let slow_pass = tokio::spawn(async move {
        let outcome = slow_engine.sync(slow.1).await;
        (outcome, Instant::now())
    });

    // Let the slow account get genuinely in flight rather than merely spawned;
    // otherwise "the fast one finished first" could be true because the slow
    // one had not started.
    tokio::time::sleep(Duration::from_millis(50)).await;

    fast_engine
        .sync(fast.1)
        .await
        .expect("the fast account syncs");
    let fast_finished = Instant::now();

    let (slow_outcome, slow_finished) = slow_pass.await.expect("the slow task");
    slow_outcome.expect("the slow account syncs too, eventually");

    // Ordering, not duration. Anything that serializes the two makes the slow
    // pass hold what it holds until it is done, so the fast pass cannot finish
    // first. Under real concurrency the fast one does, because the slow one
    // waits out `SLOW` on every call — and that stays true however loaded the
    // machine is, since both accounts are loaded equally.
    assert!(
        fast_finished < slow_finished,
        "the fast account did not finish until after the slow one — it waited \
         for it, so something is serializing the two"
    );

    // ── and each row reflects only its own pass ──────────────────────────
    let connection = database.connection().expect("a connection");
    let sync_state = SyncStateRepository::new(&connection);
    let mailboxes = MailboxRepository::new(&connection);

    for (account, inbox) in [slow, fast] {
        for folder in mailboxes.list_for_account(account).expect("its folders") {
            assert_eq!(
                folder.account_id, account,
                "a folder was filed under an account that does not own it"
            );
        }
        let state = sync_state
            .get(inbox)
            .expect("its sync state")
            .expect("a pass was recorded for this account's own inbox");
        assert_eq!(
            state.mailbox_id, inbox,
            "a sync_state row belongs to the mailbox its pass covered, not to \
             whichever account happened to finish last"
        );
    }
}
