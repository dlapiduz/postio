//! What a `UIDVALIDITY` has to do before Postio acts on it.
//!
//! A `UIDVALIDITY` change is the most expensive sentence a server can say to a
//! mail client: every cached UID for the folder becomes meaningless, so the
//! mailbox is emptied and refetched whole. On a 60,000-message folder that is
//! not a cost to pay on a server's first attempt at an answer.
//!
//! A live account on 2026-09-13 paid it for eleven folders at once (#1538).
//! The first reading was that generations were being *crossed* between
//! folders; a debug run withdrew it, showing the server hands one
//! `UIDVALIDITY` to several different folders -- which RFC 3501 permits, since
//! it asks for uniqueness per mailbox over time, not across mailboxes. What
//! was left is a server contradicting itself from one `SELECT` to the next,
//! in the same minute that three others carried no `UIDVALIDITY` at all.
//!
//! So a renumber now has to be said twice, and an omission gets one more ask.
//! The two halves that matter are that a flake is survived and a real renumber
//! still gets through; the first two cases below are the older ones, kept as
//! the record of what was eliminated.

use std::sync::Arc;

use postio_account::backend::SelectMode;
use postio_account::imap::{ConnectionPool, PoolConfig, Priority, RustlsConnector, select};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{Quirk, TestMailbox, TestServer};
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

// ---------------------------------------------------------------------------
// Confirming a renumber before acting on it (#1538)
// ---------------------------------------------------------------------------

async fn pool_over(server: &TestServer) -> ConnectionPool {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");
    ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    )
}

/// One wrong answer must not condemn a mailbox.
///
/// A `UIDVALIDITY` change is the most expensive thing a server can tell
/// Postio: every cached UID for the folder becomes meaningless and the whole
/// mailbox is refetched. On a 60,000-message folder that is not a cost to pay
/// on a server's first attempt at an answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_contradicts_itself_does_not_lose_the_mailbox() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;

    // What the pool believes, established honestly.
    select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect("the first select");

    // Now one bad answer, then the truth again.
    server.quirk(Quirk::UidValidityFlapsOnce {
        generation: 999_999,
    });

    let selected = select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect(
            "a single contradicted UIDVALIDITY condemned the mailbox. The \
             second SELECT reported the original generation, so nothing was \
             renumbered and there is nothing to resync",
        );
    assert_eq!(
        selected.generation.get(),
        1_000_001,
        "the flap was believed rather than the truth that followed it"
    );
}

/// And the other half: a renumber the server stands behind is still acted on.
///
/// The confirmation must not become a way of never believing bad news —
/// a `UIDVALIDITY` that really has changed has to reach the sync layer, which
/// is the whole reason the check exists.
#[tokio::test(flavor = "multi_thread")]
async fn a_renumber_the_server_repeats_is_still_a_renumber() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;

    select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect("the first select");

    // Not a flap: the server's answer, from here on.
    server.set_uid_validity("INBOX", UidValidity::new(2_000_002));

    let error = select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect_err("a real renumber must still be refused");
    assert!(
        matches!(
            error,
            postio_account::backend::BackendError::UidValidityChanged { .. }
        ),
        "a renumber the server repeats came back as {error} rather than \
         UidValidityChanged, so the mailbox would never be resynchronised"
    );
}

/// A `SELECT` that forgets `UIDVALIDITY` is asked again, not given up on.
#[tokio::test(flavor = "multi_thread")]
async fn a_select_that_omits_uidvalidity_is_asked_again() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;
    server.quirk(Quirk::UidValidityOmittedOnce);

    let selected = select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect("one omitted UIDVALIDITY lost the folder for this pass");
    assert_eq!(selected.generation.get(), 1_000_001);
}

/// And a server that never sends one is still a protocol error.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_sends_uidvalidity_is_still_refused() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;
    server.quirk(Quirk::UidValidityAlwaysOmitted);

    // Also what notices if the retry ever grows into a loop: against a server
    // that never answers, a bounded retry returns and an unbounded one hangs.
    let error = select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect_err("a mailbox with no UIDVALIDITY cannot be addressed by UID");
    assert!(
        matches!(
            error,
            postio_account::backend::BackendError::Protocol { .. }
        ),
        "expected a protocol refusal, got {error}"
    );
}
