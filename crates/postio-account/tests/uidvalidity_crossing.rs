//! Selecting several mailboxes in turn: does each one report its own
//! `UIDVALIDITY`, or the one before it?
//!
//! A real account on 2026-09-13 produced a whole mailbox list of
//! `UidValidityChanged`, and the numbers gave the shape away: what each
//! mailbox believed it had seen was the value of the mailbox synced *before*
//! it. A `UIDVALIDITY` misattributed this way is not cosmetic -- it is what
//! tells Postio to throw a folder's UID space away and resync it whole.
//!
//! Every value below is a distinct, recognisable number, so a crossed pair
//! names itself in the failure.
//!
//! **Both cases pass, and that is the finding they record** (#1538). They were
//! written from a field failure and did not reproduce it, so what they pin
//! down is the ground already eliminated: sequential selects on one pooled
//! connection, and the engine's real two-at-a-time shape, both attribute
//! correctly. The structural difference left over is the watcher -- fifteen
//! folders on the watch lane, sharing one `Arc<Generations>` with the general
//! lane -- which is where #1538 says to look next. Kept as regression cover
//! either way: this is an invariant worth holding whatever turns out to break
//! it.

use std::sync::Arc;

use postio_account::backend::SelectMode;
use postio_account::imap::{ConnectionPool, PoolConfig, Priority, RustlsConnector, select};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{TestMailbox, TestServer};
use postio_model::UidValidity;

const FOLDERS: [(&str, u32); 6] = [
    ("INBOX", 1_000_001),
    ("Archive", 1_000_002),
    ("Sent Messages", 1_000_003),
    ("Junk", 1_000_004),
    ("Deleted Messages", 1_000_005),
    ("Notes", 1_000_006),
];

#[tokio::test(flavor = "multi_thread")]
async fn each_mailbox_reports_its_own_uidvalidity_not_the_previous_ones() {
    let mut builder = TestServer::builder();
    for (path, validity) in FOLDERS {
        builder = builder.mailbox(TestMailbox::new(path).uid_validity(UidValidity::new(validity)));
    }
    let server = builder.start().await;

    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");
    let pool = ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    );

    // Once through, in order, on whatever connection the pool hands out --
    // which is the same one every time, because each is given back before the
    // next is asked for. Exactly what a sync pass over a folder list does.
    for (path, expected) in FOLDERS {
        let selected = select(&pool, path, SelectMode::ReadWrite, Priority::Background)
            .await
            .unwrap_or_else(|error| panic!("selecting {path}: {error}"));
        assert_eq!(
            selected.generation.get(),
            expected,
            "selecting {path} reported UIDVALIDITY {} when the server holds \
             {expected} for it. That is another mailbox's generation, and \
             acting on it resyncs a folder that never changed.",
            selected.generation.get()
        );
    }

    // And again, which is where the real account failed: the second pass is
    // the one that compares what it sees against what it remembers.
    for (path, expected) in FOLDERS {
        let selected = select(&pool, path, SelectMode::ReadWrite, Priority::Background)
            .await
            .unwrap_or_else(|error| panic!("selecting {path} a second time: {error}"));
        assert_eq!(selected.generation.get(), expected, "second pass, {path}");
    }
}

/// The same folders, selected two at a time — which is how the engine syncs
/// them, and the only difference between this and the passing case above.
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_selects_do_not_cross_their_uidvalidities() {
    let mut builder = TestServer::builder();
    for (path, validity) in FOLDERS {
        builder = builder.mailbox(TestMailbox::new(path).uid_validity(UidValidity::new(validity)));
    }
    let server = builder.start().await;

    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");
    let pool = Arc::new(ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    ));

    // Three rounds, because the failure on the real account needed a first
    // pass to fill the generation log before a second could disagree with it.
    for round in 0..3 {
        for pair in FOLDERS.chunks(2) {
            let mut running = Vec::new();
            for (path, expected) in pair {
                let pool = Arc::clone(&pool);
                let path = *path;
                let expected = *expected;
                running.push(tokio::spawn(async move {
                    let selected = select(&pool, path, SelectMode::ReadWrite, Priority::Background)
                        .await
                        .unwrap_or_else(|error| panic!("round {round}, {path}: {error}"));
                    assert_eq!(
                        selected.generation.get(),
                        expected,
                        "round {round}: {path} reported UIDVALIDITY {} rather \
                         than its own {expected}",
                        selected.generation.get()
                    );
                }));
            }
            for task in running {
                task.await.expect("a select task");
            }
        }
    }
}
