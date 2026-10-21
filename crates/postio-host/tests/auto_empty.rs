//! A host that starts its idle passes empties Junk and Trash as configured
//! (#1932): old mail is purged through the queue, Flagged and young mail is
//! left, and with the setting off (the default) nothing is.

use std::time::{Duration, Instant};

use chrono::{Duration as Days, Utc};
use postio_core::retention::AutoEmpty;
use postio_host::Host;
use postio_model::{Flag, MailboxRole, Message, Operation};
use postio_storage::repository::{MailboxRepository, MessageRepository, OperationQueueRepository};
use postio_storage::{BlobStore, test_support};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a test runtime")
}

async fn purges(database: &postio_storage::Store, account: postio_model::AccountId) -> usize {
    let connection = database.connect().await.expect("checkout");
    OperationQueueRepository::new(&connection)
        .pending(account, Utc::now())
        .await
        .expect("queue")
        .iter()
        .filter(|row| matches!(row.operation, Operation::Purge { .. }))
        .count()
}

fn start(
    policy: AutoEmpty,
) -> (
    tokio::runtime::Runtime,
    postio_storage::Store,
    postio_model::AccountId,
    Host,
    tempfile::TempDir,
) {
    let rt = runtime();
    let database = rt.block_on(test_support::memory());
    let account = rt.block_on(async {
        let connection = database.connect().await.expect("checkout");
        let (account, _) = test_support::account_with_inbox(&connection).await;
        let mut trash = test_support::mailbox(&connection, &account, "Trash").await;
        trash.role = MailboxRole::Trash;
        MailboxRepository::new(&connection)
            .update(&trash)
            .await
            .expect("role");
        for (days, flagged) in [(60, false), (60, false), (60, true), (2, false)] {
            let mut message = Message::new(account.id, trash.id, Utc::now() - Days::days(days));
            if flagged {
                message.flags.insert(Flag::Flagged);
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("filed");
        }
        account.id
    });
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf(), &test_support::blob_keys())
        .expect("a blob store");
    let host = Host::start(database.clone(), blobs, |wiring| {
        wiring.with_auto_empty(policy)
    })
    .expect("a host");
    (rt, database, account, host, directory)
}

#[test]
fn the_idle_passes_purge_old_unflagged_mail_from_trash() {
    let (rt, database, account, host, _blobs) = start(AutoEmpty {
        junk_days: 0,
        trash_days: 30,
    });
    host.start_idle_passes();
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_secs(60));
    while rt.block_on(purges(&database, account)) < 2 {
        assert!(
            Instant::now() < deadline,
            "the pass never queued the purges"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        rt.block_on(purges(&database, account)),
        2,
        "not the Flagged one, not the two-day-old one"
    );
}

#[test]
fn off_by_default_leaves_the_trash_alone() {
    let (rt, database, account, host, _blobs) = start(AutoEmpty::default());
    host.start_idle_passes();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rt.block_on(purges(&database, account)), 0);
}
