use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_core::bridge::{EventStream, event_channel};
use postio_core::state::{AppState, SharedState};
use postio_core::{Command, MessageTarget};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_session::actions::Actions;
use postio_storage::repository::{
    AccountRepository, ListQuery, ListScope, MailboxRepository, MessageRepository,
};
use postio_storage::test_support::TempStore;
use postio_storage::{BlobStore, Store, test_support};

const LATENCY: Duration = Duration::from_millis(40);

const BULK: &str = "Lists";
const BULK_MESSAGES: u32 = 3_000;

const MOST_BACKFILL_A_KEYSTROKE_MAY_WAIT_FOR: u32 = 100;

fn message(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         To: Postio <postio@example.net>\r\n\
         Subject: message {n}\r\n\
         Message-ID: <m-{n}@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
         \r\n\
         Body {n}.\r\n"
    )
    .into_bytes()
}

fn folder(path: &str, attributes: &[&str], messages: u32) -> MockMailbox {
    let mut mailbox = MockMailbox::new(path);
    if !attributes.is_empty() {
        mailbox = mailbox.attributes(attributes.iter().copied());
    }
    for n in 1..=messages {
        mailbox = mailbox.message(MockMessage::new(message(n)));
    }
    mailbox
}

async fn engine_over(backend: Arc<MockBackend>) -> (TempStore, Engine, tempfile::TempDir) {
    let database = test_support::temp().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        test_support::account(&connection).await
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
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
    .expect("the engine starts");
    (database, engine, directory)
}

async fn mailbox_at(database: &Store, path: &str) -> Option<postio_model::Mailbox> {
    let connection = database.connect().await.ok()?;
    let account = AccountRepository::new(&connection)
        .list()
        .await
        .ok()?
        .into_iter()
        .next()?;
    MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .ok()?
        .into_iter()
        .find(|mailbox| mailbox.path == path)
}

async fn stored(database: &Store, path: &str) -> u32 {
    let Some(mailbox) = mailbox_at(database, path).await else {
        return 0;
    };
    let Ok(connection) = database.connect().await else {
        return 0;
    };
    MessageRepository::new(&connection)
        .count(&ListQuery {
            scope: ListScope::Mailbox(mailbox.id),
            limit: 0,
            after: None,
        })
        .await
        .unwrap_or(0)
}

async fn until<F, Fut>(what: &str, condition: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let waited = tokio::time::timeout(Duration::from_secs(180), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_archive_keystroke_does_not_wait_for_the_backfill() {
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 20))
            .mailbox(folder("Archive", &["\\Archive"], 5))
            .mailbox(folder(BULK, &[], BULK_MESSAGES))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine, _directory) = engine_over(backend.clone()).await;

    until("INBOX and Archive to arrive", async || {
        stored(&database, "INBOX").await > 0 && mailbox_at(&database, "Archive").await.is_some()
    })
    .await;

    let inbox = mailbox_at(&database, "INBOX").await.expect("an INBOX");
    let archive = mailbox_at(&database, "Archive").await.expect("an Archive");
    let subject = {
        let connection = database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox.id),
                limit: 1,
                after: None,
            })
            .await
            .expect("a page")
            .into_iter()
            .next()
            .expect("a message in the inbox")
    };

    until("the bulk backfill to be under way", async || {
        stored(&database, BULK).await > 200
    })
    .await;

    let state = SharedState::default();
    let (quiet, _quiet_events): (_, EventStream) = event_channel();
    state.update(&quiet, |app: &mut AppState| app.open_mailbox(inbox.id));
    state.update(&quiet, |app: &mut AppState| {
        app.select(Vec::new(), Some(subject.id))
    });
    let actions = Actions::new((*database).clone(), state);
    let (events, _stream): (_, EventStream) = event_channel();

    let backfill_before = stored(&database, BULK).await;
    let started = Instant::now();
    actions
        .run(
            &Command::Archive {
                target: MessageTarget::Selection,
            },
            &events,
        )
        .await
        .expect("the archive");
    let took = started.elapsed();
    let backfill_after = stored(&database, BULK).await;

    let landed = {
        let connection = database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .get(subject.id)
            .await
            .expect("a read")
            .expect("the message")
            .mailbox_id
    };
    assert_eq!(landed, archive.id, "the archive did not actually file it");

    assert!(
        backfill_after < BULK_MESSAGES,
        "the bulk backfill finished before the keystroke was measured, so this \
         run never exercised the contention it is about ({backfill_after} of \
         {BULK_MESSAGES} stored)"
    );

    let waited_for = backfill_after.saturating_sub(backfill_before);
    eprintln!(
        "the archive waited for {waited_for} messages of backfill ({took:?}); \
         the bound is {MOST_BACKFILL_A_KEYSTROKE_MAY_WAIT_FOR}"
    );
    assert!(
        waited_for <= MOST_BACKFILL_A_KEYSTROKE_MAY_WAIT_FOR,
        "the archive waited for {waited_for} messages of backfill to be written \
         ahead of it ({took:?} of wall clock). The write gate is supposed to \
         bound that at one write unit per sync lane. Starved — which is what \
         #425 was — this number ran into the thousands."
    );

    drop(engine);
}
