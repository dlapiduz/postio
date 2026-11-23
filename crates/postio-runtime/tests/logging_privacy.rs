//! No level, ever, puts message content in the log.
//!
//! POSTIO-OWN-BINARY: it installs the global tracing subscriber.
//!
//! The forbidden strings are read back out of the seeded store, so a new fixture is covered without
//! touching this file.
//!
//! POSTIO-MEASUREMENT: it runs a corpus through the engine at every log
//! level, 187 s, the longest case in this crate, so it runs on the nightly
//! timer. It is a privacy gate: a red run there blocks the merge path like
//! any other nightly failure.

use std::sync::Arc;

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_model::MailboxRole;
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{ListQuery, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

use postio_test_support::logs::Captured;

/// Previews are trimmed to a distinctive prefix: a whole one could be wrapped or truncated by a
/// formatter and the test pass by accident.
async fn content_of(
    database: &postio_storage::Store,
    account: postio_model::AccountId,
) -> Vec<String> {
    let connection = database.connect().await.expect("a connection");
    let rows = MessageRepository::new(&connection)
        .page(&ListQuery::account(account).limit(500))
        .await
        .expect("reading the seeded mail");
    assert!(!rows.is_empty(), "the fixture seeded no mail to protect");

    let mut secrets = Vec::new();
    // The account's own address is the user's identity: an error naming it is wrong in a log.
    {
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .list_enabled()
            .await
            .expect("reading the account")
            .into_iter()
            .next()
            .expect("the fixture made one");
        secrets.push(account.address.address.clone());
    }
    for row in rows {
        if let Some(subject) = row.subject.filter(|s| s.trim().len() >= 8) {
            secrets.push(subject.trim().to_string());
        }
        if let Some(preview) = row.preview.filter(|p| p.trim().len() >= 24) {
            secrets.push(preview.trim().chars().take(24).collect());
        }
        if let Some(from) = row.from {
            secrets.push(from.address.clone());
            if let Some(name) = from.name.clone().filter(|n| n.trim().len() >= 5) {
                secrets.push(name);
            }
        }
    }
    secrets.sort();
    secrets.dedup();
    secrets
}

fn server() -> MockBackend {
    let raw = b"From: Ada Lovelace <ada@example.com>\r\n\
                To: Postio <postio@example.net>\r\n\
                Subject: a message the server holds\r\n\
                Message-ID: <held-1@example.com>\r\n\
                Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
                \r\n\
                The bytes that had to travel to get here.\r\n"
        .to_vec();
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(MockMessage::new(raw)))
        .build()
}

#[tokio::test]
async fn no_message_content_reaches_the_log_at_any_level() {
    let captured = Captured::default();

    // No filter, at `trace`. Global, not `set_default`: the engine's thread would otherwise have no
    // subscriber and the test would assert over an empty buffer.
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("this test binary runs one test and owns the subscriber");

    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let secrets = content_of(&database, report.account.id).await;

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = postio_core::bridge::event_channel();

    let engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: Arc::new(server()),
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
    .expect("the engine starts");

    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let _ = engine.drain().await;
    let _ = engine.sync(inbox).await;
    let _ = engine.seed_backfill(inbox, 50).await;
    drop(engine);

    let log = captured.text();
    // Proves the paths ran and were observed; an engine that logged nothing would pass otherwise.
    for expected in ["engine{account=", ":drain:", ":sync{"] {
        assert!(
            log.contains(expected),
            "the log has no `{expected}` span, so the sync path was never \
             observed and this test proves nothing:\n{log}"
        );
    }

    let leaked: Vec<&String> = secrets
        .iter()
        .filter(|secret| log.contains(secret.as_str()))
        .collect();

    assert!(
        leaked.is_empty(),
        "{} of the store's own subjects, previews or sender addresses reached \
         the log. Log ids, counts, folder paths, durations and outcomes — never \
         what a message says or who sent it. Leaked: {:?}",
        leaked.len(),
        leaked
    );
}
