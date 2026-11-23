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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Handed {
    id: MessageId,
    subject: Option<String>,
    role: MailboxRole,
}

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

fn newsletter(n: u32) -> Vec<u8> {
    format!(
        "From: Ledger <news@ledger.example>\r\n\
         List-Unsubscribe: <https://ledger.example/u/{n}>\r\n\
         Precedence: bulk\r\n\
         Subject: Issue {n}\r\n\r\nThe numbers.\r\n"
    )
    .into_bytes()
}

#[derive(Debug, Default)]
struct Reader {
    said: Mutex<Vec<Option<postio_model::promoted::PromotedHeaders>>>,
}

#[async_trait::async_trait]
impl FilingPass for Reader {
    async fn file(
        &self,
        _transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        self.said
            .lock()
            .expect("not poisoned")
            .extend(filed.iter().map(|filed| filed.message.promoted));
        Ok(FilingEffects::default())
    }
}

#[tokio::test]
async fn an_arrival_is_filed_knowing_its_promoted_headers_and_a_first_sync_is_not() {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    mailbox = mailbox.message(MockMessage::new(newsletter(1)));
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inbox = local(&connection).await;
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");

    backend
        .append(INBOX, &AppendMessage::new(newsletter(2)))
        .await
        .expect("deliver");
    let reader = Reader::default();
    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&reader),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("an incremental pass");
    let Outcome::Incremental { arrived, .. } = outcome else {
        panic!("expected an incremental pass, got {outcome:?}");
    };

    let bulk = postio_model::promoted::PromotedHeaders {
        unsubscribe_offered: true,
        automation: postio_model::promoted::PRECEDENCE_BULK,
    };
    assert_eq!(
        *reader.said.lock().expect("not poisoned"),
        vec![Some(bulk)],
        "the filing pass is handed what the arrival said"
    );
    let stored = |id| {
        let connection = &connection;
        async move {
            postio_storage::repository::MessageRepository::new(connection)
                .get(id)
                .await
                .expect("a read")
                .expect("the message")
                .promoted
        }
    };
    assert_eq!(
        stored(arrived[0]).await,
        Some(bulk),
        "and the store keeps it"
    );
    let first: Vec<(MessageId, Option<String>)> = postio_storage::sql::all(
        &connection,
        "SELECT id, subject FROM messages WHERE subject = 'Issue 1'",
        (),
        |row| {
            Ok((
                MessageId::new(postio_storage::sql::RowExt::col(row, 0)?),
                postio_storage::sql::RowExt::col(row, 1)?,
            ))
        },
    )
    .await
    .expect("the first sync's message");
    assert_eq!(
        stored(first[0].0).await,
        None,
        "the first sync did not ask, so does not know"
    );
}

#[derive(Debug, Default)]
struct FilesThenFails;

#[async_trait::async_trait]
impl FilingPass for FilesThenFails {
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        use postio_storage::repository::{
            FilterDecision, FilterDecisionRepository, FilterLayer, FilterReason,
        };
        FilterDecisionRepository::new(transaction)
            .record(&FilterDecision {
                message: filed[0].message.id,
                reason: FilterReason::Promotion,
                source: None,
                layer: FilterLayer::Header,
                decided_at: chrono::Utc::now(),
            })
            .await?;
        Err(postio_storage::Error::NotPersisted {
            entity: "a filing probe's decision",
        }
        .into())
    }
}

#[tokio::test]
async fn a_filing_pass_that_fails_loses_no_mail() {
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

    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&FilesThenFails),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("the pass files the mail whatever the filing pass says");

    let Outcome::Incremental { arrived, .. } = outcome else {
        panic!("expected an incremental pass, got {outcome:?}");
    };
    assert_eq!(arrived.len(), 1, "one delivery");
    let stored = postio_storage::repository::MessageRepository::new(&connection)
        .get(arrived[0])
        .await
        .expect("a read")
        .expect("the arrival was stored");
    assert_eq!(stored.mailbox_id, inbox.id, "and it is in the inbox");
    assert_eq!(
        postio_storage::repository::FilterDecisionRepository::new(&connection)
            .get(arrived[0])
            .await
            .expect("a read"),
        None,
        "the failed pass's own write was taken back"
    );

    let probe = Probe::default();
    resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&probe),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("the next pass");
    assert!(probe.calls().is_empty(), "{:?}", probe.calls());
}

async fn modseq_less_server_with_messages(count: u32) -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder()
        .capabilities(["IMAP4rev1"])
        .mailbox(mailbox)
        .build();
    backend.connect().await.expect("connect");
    backend
}

#[tokio::test]
async fn a_backend_without_modseq_hands_the_filing_pass_what_arrived() {
    let backend = modseq_less_server_with_messages(2).await;
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
    .expect("a modseq-less pass");
    assert!(
        matches!(
            outcome,
            Outcome::Full {
                reason: postio_model::FullResyncReason::NoModSeq,
                ..
            }
        ),
        "the fixture is about the modseq-less plan: {outcome:?}"
    );

    let calls = probe.calls();
    assert_eq!(calls.len(), 1, "one call, for the one unit: {calls:?}");
    assert_eq!(calls[0].len(), 1, "the arrival and nothing else: {calls:?}");
    assert_eq!(calls[0][0].subject.as_deref(), Some("Note 3"));
    assert_eq!(calls[0][0].role, MailboxRole::Inbox);

    let again = Probe::default();
    resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&again),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("a pass with nothing new");
    assert!(again.calls().is_empty(), "{:?}", again.calls());
}

#[derive(Debug, Default)]
struct Classifies {
    threads: Mutex<Vec<Option<postio_model::ThreadId>>>,
}

struct Strangers;

impl postio_classify::Facts for Strangers {
    fn wrote_to(&self, _: &postio_model::EmailAddress) -> bool {
        false
    }
    fn took_part(&self, _: postio_model::ThreadId) -> bool {
        false
    }
    fn own_domain(&self, _: &postio_model::EmailAddress) -> bool {
        false
    }
    fn never_filter(&self, _: &postio_model::EmailAddress) -> bool {
        false
    }
}

struct Shipped;

impl postio_classify::Rules for Shipped {
    fn senders(&self) -> &postio_classify::Senders {
        postio_classify::Senders::shipped()
    }
}

#[async_trait::async_trait]
impl FilingPass for Classifies {
    async fn file(
        &self,
        _transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        for message in filed {
            let _ = postio_classify::at_filing(message, &Strangers, &Shipped);
            self.threads
                .lock()
                .expect("not poisoned")
                .push(message.thread);
        }
        Ok(FilingEffects::default())
    }
}

#[tokio::test]
async fn the_classifier_is_handed_what_sync_filed_with_its_thread() {
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

    let pass = Classifies::default();
    let outcome = resync_mailbox_filing(
        &connection,
        &backend,
        &inbox,
        Some(&pass),
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("an incremental pass");
    let Outcome::Incremental { arrived, .. } = outcome else {
        panic!("expected an incremental pass, got {outcome:?}");
    };

    let stored = postio_storage::repository::MessageRepository::new(&connection)
        .get(arrived[0])
        .await
        .expect("a read")
        .expect("the arrival");
    assert!(stored.thread_id.is_some(), "threading placed the arrival");
    assert_eq!(
        *pass.threads.lock().expect("not poisoned"),
        vec![stored.thread_id],
        "the pass was handed the conversation the arrival joined"
    );
}
