//! A flag somebody changed on the server has to reach the *list*, not just
//! the store (#1176).
//!
//! The reported symptom is a message marked read in another client staying
//! bold in Postio until a manual refresh. `postio-sync`'s
//! `a_server_side_flag_change_and_deletion_both_reflect_locally` already
//! proves the pass reads the new flag and writes it down, so the store is not
//! where this goes wrong. What nothing covered is the step after: whether the
//! engine *says* anything about it.
//!
//! That is the difference between a row that repaints and a row that does
//! not, and it is invisible to every test that stops at the store. This
//! project's characteristic bug is layers that each pass and are not joined
//! up, and this is the join.
//!
//! # It was checked for vacuity
//!
//! A test that asserts "an event was emitted" passes very easily by accident:
//! the first pass emits one too, and a drain that misses it by a moment would
//! make this green for the wrong reason. So the same case was run once with
//! the flag change removed and nothing else altered, and the second pass then
//! emitted only `ConnectionChanged`:
//!
//! ```text
//!   with the flag change:  [ConnectionChanged, MessageListChanged, ConnectionChanged, ...]
//!   without it:            [ConnectionChanged, ConnectionChanged]
//! ```
//!
//! So the announcement below is caused by the flag and by nothing else.

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

    let database = test_support::memory();
    let (account, inbox) = {
        let connection = database.connection().expect("a connection");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, INBOX);
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

    // The mailbox as it stands, and everything it had to say about getting
    // there is not what this is about.
    engine.sync(inbox.id).await.expect("the first pass");
    let _ = drain(&events);

    // Another client marks message 2 read.
    backend
        .store_flags(
            INBOX,
            &[rid(2)],
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("another client marks it read");

    engine.sync(inbox.id).await.expect("the pass that sees it");

    // The store heard, which is the part already covered elsewhere. Asserted
    // here so a failure below cannot be read as "the pass did nothing".
    let stored = {
        let connection = database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .page(&ListQuery::mailbox(inbox.id))
            .expect("list the mailbox")
    };
    let seen: Vec<bool> = stored.iter().map(|row| row.seen).collect();
    assert!(
        seen.iter().any(|seen| *seen),
        "the pass never wrote the flag down, so this case cannot say anything \
         about what it announced: {seen:?}"
    );

    // And the list was told. Without this the row stays bold until something
    // else happens to reload it, which is the whole of #1176.
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
