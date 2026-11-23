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

    for (path, expected) in FOLDERS {
        let selected = select(&pool, path, SelectMode::ReadWrite, Priority::Background)
            .await
            .unwrap_or_else(|error| panic!("selecting {path} a second time: {error}"));
        assert_eq!(selected.generation.get(), expected, "second pass, {path}");
    }
}

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

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_contradicts_itself_does_not_lose_the_mailbox() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;

    select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect("the first select");

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

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_sends_uidvalidity_is_still_refused() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;
    server.quirk(Quirk::UidValidityAlwaysOmitted);

    // A bounded retry returns against a server that never answers; an unbounded one hangs.
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

/// A lost `UIDVALIDITY` is retried; one the server never sends is not. `io-imap` drops undecodable
/// untagged lines and completes `Ok`, so both leave the same hole.
/// The skip count tells them apart: a lost line discards the connection, a never-sent one must not
/// retry for ever.
#[tokio::test(flavor = "multi_thread")]
async fn a_uidvalidity_lost_in_transit_is_transient() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX").uid_validity(UidValidity::new(1_000_001)))
        .start()
        .await;
    let pool = pool_over(&server).await;
    server.quirk(Quirk::UidValidityLostToAnUndecodableLine);

    let error = select(&pool, "INBOX", SelectMode::ReadWrite, Priority::Background)
        .await
        .expect_err("a SELECT with no readable UIDVALIDITY cannot be used");
    assert!(
        error.is_transient(),
        "a line the client lost must be retried on a fresh connection, not \
         reported to the user as the fate of what they asked for. Got: {error}"
    );
}
