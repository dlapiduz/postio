//! The filing pass (spec 007, `contracts/engine.md`, "The filing pass").
//!
//! A host in Focus mode hands every incremental pass a filing pass, and the
//! pass hands it what arrived, inside the transaction that filed it. Nothing
//! a first sync files is new mail -- years of inbox, not arrivals -- so
//! nothing a first sync files reaches it, however the first sync is reached.

use std::sync::Mutex;

use postio_account::backend::{
    AppendMessage, FlagChange, MailBackend, MockBackend, MockMailbox, MockMessage,
};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, Mailbox, MailboxRole, MessageId, UidValidity};
use postio_storage::Connection;
use postio_storage::test_support;
use postio_sync::{
    FiledMessage, FilingEffects, FilingPass, Outcome, SyncError, resync_mailbox_filing,
    sync_mailbox, sync_mailbox_with_batch_size,
};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

async fn server_with_messages(count: u32) -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");
    backend
}

async fn local(connection: &Connection) -> Mailbox {
    let account = test_support::account(connection).await;
    test_support::mailbox(connection, &account, INBOX).await
}

/// One message as the probe was handed it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Handed {
    id: MessageId,
    subject: Option<String>,
    role: MailboxRole,
}

/// A filing pass that files nothing and remembers every call.
#[derive(Debug, Default)]
struct Probe {
    calls: Mutex<Vec<Vec<Handed>>>,
}

impl Probe {
    fn calls(&self) -> Vec<Vec<Handed>> {
        self.calls.lock().expect("not poisoned").clone()
    }
}

#[async_trait::async_trait]
impl FilingPass for Probe {
    async fn file(
        &self,
        _transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        self.calls.lock().expect("not poisoned").push(
            filed
                .iter()
                .map(|filed| Handed {
                    id: filed.message.id,
                    subject: filed.message.subject.clone(),
                    role: filed.role,
                })
                .collect(),
        );
        Ok(FilingEffects::default())
    }
}

#[tokio::test]
async fn an_incremental_pass_hands_the_filing_pass_exactly_what_arrived() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inbox = local(&connection).await;
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");

    backend
        .append(INBOX, &AppendMessage::new(note(3)))
        .await
        .expect("deliver");
    let probe = Probe::default();
    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&probe),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("an incremental pass");

    let Outcome::Incremental { arrived, .. } = outcome else {
        panic!("expected an incremental pass, got {outcome:?}");
    };
    assert_eq!(arrived.len(), 1, "one delivery");
    assert_eq!(
        probe.calls(),
        vec![vec![Handed {
            id: arrived[0],
            subject: Some("Note 3".to_owned()),
            role: MailboxRole::Inbox,
        }]],
        "the arrival, once, as the store filed it and where"
    );
}

#[tokio::test]
async fn a_pass_that_brings_nothing_new_files_nothing() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inbox = local(&connection).await;
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");

    // Another client reads a message: a change, not an arrival.
    backend
        .store_flags(
            INBOX,
            &[postio_model::RemoteId::new(format!("{VALIDITY}:1"))],
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");
    let probe = Probe::default();
    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&probe),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("an incremental pass");

    assert!(
        matches!(outcome, Outcome::Incremental { changed: 1, ref arrived, .. } if arrived.is_empty()),
        "{outcome:?}"
    );
    assert!(probe.calls().is_empty(), "{:?}", probe.calls());
}

#[tokio::test]
async fn a_first_sync_resumed_through_the_incremental_entry_files_nothing() {
    // A first pass cut short leaves a sync state that never completed, so
    // the next pass comes through the same call an incremental one does and
    // plans a full enumeration. Everything it stores is the backlog.
    let backend = server_with_messages(6).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inbox = local(&connection).await;
    let cancel = CancelToken::new();
    let _ = sync_mailbox_with_batch_size(&connection, &backend, &inbox, 1, &cancel, |_| {
        cancel.cancel();
    })
    .await;

    let probe = Probe::default();
    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&probe),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("the resumed first sync");

    assert!(
        matches!(outcome, Outcome::Full { .. }),
        "the fixture needs a first sync resumed: {outcome:?}"
    );
    assert_eq!(
        postio_storage::repository::MessageRepository::new(&connection)
            .uids_in(inbox.id, postio_model::Generation::new(VALIDITY))
            .await
            .expect("uids")
            .len(),
        6,
        "the first sync finished"
    );
    assert!(
        probe.calls().is_empty(),
        "a first sync filed its backlog: {:?}",
        probe.calls()
    );
}
