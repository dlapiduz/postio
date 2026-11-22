use std::sync::Arc;

use postio_account::backend::{FlagChange, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_core::Event;
use postio_core::bridge::{EventStream, event_channel};
use postio_model::{Flag, FlagSet, RemoteId};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{ListQuery, MessageRepository};
use postio_storage::{BlobStore, test_support};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn note(n: u32) -> Vec<u8> {
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

fn rid(uid: u32) -> RemoteId {
    RemoteId::new(format!("{VALIDITY}:{uid}"))
}

fn drain(events: &EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(event);
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn a_flag_changed_on_the_server_is_announced_and_not_only_stored() {
    let mut inbox_mock =
        MockMailbox::new(INBOX).uid_validity(postio_model::UidValidity::new(VALIDITY));
    for n in 1..=3 {
        inbox_mock = inbox_mock.message(MockMessage::new(note(n)));
    }
    let backend = Arc::new(MockBackend::builder().mailbox(inbox_mock).build());

    let database = test_support::memory().await;
    let (account, inbox) = {
        let connection = database.connect().await.expect("a connection");
        let account = test_support::account(&connection).await;
        let inbox = test_support::mailbox(&connection, &account, INBOX).await;
        (account, inbox)
    };

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
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

    engine.sync(inbox.id).await.expect("the first pass");
    let _ = drain(&events);

    backend
        .store_flags(
            INBOX,
            &[rid(2)],
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("another client marks it read");

    engine.sync(inbox.id).await.expect("the pass that sees it");

    let stored = {
        let connection = database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .page(&ListQuery::mailbox(inbox.id))
            .await
            .expect("list the mailbox")
    };
    let seen: Vec<bool> = stored.iter().map(|row| row.seen).collect();
    assert!(
        seen.iter().any(|seen| *seen),
        "the pass never wrote the flag down, so this case cannot say anything \
         about what it announced: {seen:?}"
    );

    let told = drain(&events);
    assert!(
        told.iter().any(|event| matches!(
            event,
            Event::MessageListChanged { .. } | Event::MessagesChanged { .. }
        )),
        "the flag reached the store and nothing was announced, so no row \
         repaints and the message stays bold until a manual refresh. Events: \
         {told:?}"
    );
}
