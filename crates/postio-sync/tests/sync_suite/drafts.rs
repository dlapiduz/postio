use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox};
use postio_model::{Account, Draft, EmailAddress, Identity, MailboxId, Operation, OperationTarget};
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, DraftRepository, MailboxRepository, OperationQueueRepository,
};
use postio_storage::test_support;
use postio_sync::{DrainReport, Drainer};

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
}

struct TempBlobs {
    store: BlobStore,
    directory: std::path::PathBuf,
}

impl TempBlobs {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "postio-drafts-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = BlobStore::open(&directory, &postio_storage::test_support::blob_keys())
            .expect("a blob store");
        Self { store, directory }
    }
}

impl Drop for TempBlobs {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

async fn account_with_drafts(connection: &Connection) -> (Account, MailboxId) {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    let mut identity = Identity::new(
        postio_model::ids::AccountId::UNASSIGNED,
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    identity.is_default = true;
    account.identities = vec![identity];

    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("create account");
    let drafts = test_support::mailbox(connection, &account, "Drafts").await;
    (account, drafts.id)
}

fn a_draft(account: &Account, subject: &str) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.to = vec![EmailAddress::new(None::<String>, "grace@example.net")];
    draft.subject = subject.to_owned();
    draft.body.text = Some("Half a thought, still being had.".to_owned());
    draft.start_as(&account.identities[0]);
    draft
}

async fn a_server(mailbox: &str) -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new(mailbox))
        .build();
    backend.connect().await.expect("connect");
    backend
}

async fn drain(
    connection: &Connection,
    backend: &MockBackend,
    blobs: &BlobStore,
    account: &Account,
) -> DrainReport {
    Drainer::new(backend)
        .with_blobs(blobs)
        .drain(connection, account.id, at(10))
        .await
        .expect("drain")
}

async fn exists(backend: &MockBackend, mailbox: &str) -> u32 {
    backend.status(mailbox).await.expect("status").exists
}

#[tokio::test]
async fn an_autosaved_draft_reaches_the_drafts_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;

    let mut draft = a_draft(&account, "Tide gate interlock");
    DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(9))
        .await
        .expect("save and queue");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(exists(&backend, "Drafts").await, 1);

    let stored = DraftRepository::new(&connection)
        .get(draft.id)
        .await
        .expect("get")
        .expect("the draft");
    assert!(
        stored.server.remote_id.is_some(),
        "the row learns where its copy landed, which is what lets the next \
         save replace it rather than add to it"
    );
}

#[tokio::test]
async fn editing_a_draft_replaces_its_copy_rather_than_adding_one() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    drafts.save_and_sync(&mut draft, at(9)).await.expect("save");
    drain(&connection, &backend, &blobs.store, &account).await;
    let first = drafts
        .get(draft.id)
        .await
        .expect("get")
        .expect("the draft")
        .server
        .remote_id;

    draft.body.text = Some("Half a thought, now most of one.".to_owned());
    drafts
        .save_and_sync(&mut draft, at(10))
        .await
        .expect("save again");
    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(
        exists(&backend, "Drafts").await,
        1,
        "one draft, not one per edit"
    );

    let second = drafts
        .get(draft.id)
        .await
        .expect("get")
        .expect("the draft")
        .server
        .remote_id;
    assert_ne!(first, second, "and the row follows the copy that is there");
}

#[tokio::test]
async fn a_run_of_autosaves_costs_one_round_trip() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    for (index, hour) in [7, 8, 9].into_iter().enumerate() {
        draft.subject = format!("Tide gate interlock, revision {index}");
        drafts
            .save_and_sync(&mut draft, at(hour))
            .await
            .expect("save");
    }

    let before = backend.calls();
    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(
        report.applied, 3,
        "all three rows are settled — the report counts rows, not round trips: {report:?}"
    );
    assert_eq!(
        backend.calls() - before,
        2,
        "and they cost one CAPABILITY and one APPEND between them"
    );
    assert_eq!(exists(&backend, "Drafts").await, 1);
}

#[tokio::test]
async fn discarding_a_draft_takes_the_server_copy_with_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    drafts.save_and_sync(&mut draft, at(9)).await.expect("save");
    drain(&connection, &backend, &blobs.store, &account).await;
    assert_eq!(exists(&backend, "Drafts").await, 1);

    drafts.discard(draft.id, at(10)).await.expect("discard");
    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(
        exists(&backend, "Drafts").await,
        0,
        "a draft the user threw away is not left sitting on their phone"
    );
}

#[tokio::test]
async fn a_draft_discarded_before_it_was_ever_uploaded_asks_the_server_for_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    drafts.save_and_sync(&mut draft, at(9)).await.expect("save");
    drafts.discard(draft.id, at(10)).await.expect("discard");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.obsolete, 1, "{report:?}");
    assert_eq!(report.applied, 0, "{report:?}");
    assert_eq!(exists(&backend, "Drafts").await, 0);
}

#[tokio::test]
async fn a_renumbered_drafts_mailbox_is_never_expunged_by_a_stale_uid() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Drafts").uid_validity(postio_model::UidValidity::new(2)))
        .build();
    backend.connect().await.expect("connect");

    let mut draft = a_draft(&account, "Tide gate interlock");
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft.id),
            &Operation::DiscardDraft {
                mailbox: drafts_mailbox,
                remote_id: postio_model::RemoteId::new("1:1"),
            },
            at(9),
        )
        .await
        .expect("enqueue");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.obsolete, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
}

#[tokio::test]
async fn a_draft_whose_attachment_is_still_being_written_waits_rather_than_fails() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;

    let mut draft = a_draft(&account, "Tide gate interlock");
    draft.attachments = vec![postio_model::Attachment::new(
        postio_model::ids::MessageId::UNASSIGNED,
        "application/pdf",
        1_024,
    )];
    DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(9))
        .await
        .expect("save");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.deferred, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(exists(&backend, "Drafts").await, 0);
}

#[tokio::test]
async fn the_copy_in_drafts_keeps_the_bcc_the_sent_message_will_not() {
    // Bcc stays in the Drafts copy (the user's own folder) and out of the sent bytes; different
    // bytes.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;

    let mut draft = a_draft(&account, "Tide gate interlock");
    draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];
    DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(9))
        .await
        .expect("save and queue");

    drain(&connection, &backend, &blobs.store, &account).await;

    let stored = DraftRepository::new(&connection)
        .get(draft.id)
        .await
        .expect("get")
        .expect("the draft");
    let remote_id = stored.server.remote_id.expect("the copy landed");

    let mut sink = postio_account::backend::VecSink::new();
    backend
        .fetch_body(
            "Drafts",
            &remote_id,
            &mut sink,
            &postio_account::cancel::CancelToken::new(),
        )
        .await
        .expect("read the copy back");

    let raw = String::from_utf8_lossy(sink.as_slice()).into_owned();
    assert!(
        raw.contains("quiet@example.com"),
        "the draft on the server has to carry every recipient the user typed"
    );
}

async fn listed(connection: &Connection, mailbox: MailboxId) -> Vec<String> {
    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(mailbox),
        limit: 50,
        after: None,
    };
    postio_storage::repository::MessageRepository::new(connection)
        .page(&query)
        .await
        .expect("a page of the Drafts folder")
        .into_iter()
        .map(|row| row.subject.unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn a_draft_this_client_uploaded_does_not_come_back_as_a_second_row() {
    // #166 gave the draft its own row at save, so the folder lists it before any sync; the next
    // pass must not add a second row for its own append (#51).
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let blobs = TempBlobs::new();
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new("Drafts").message(postio_account::backend::MockMessage::new(
                b"From: Ada Lovelace <ada@example.com>\r\n\
                  Subject: Written on the phone\r\n\r\nStarted elsewhere.\r\n"
                    .to_vec(),
            )),
        )
        .build();
    backend.connect().await.expect("connect");

    let mut draft = a_draft(&account, "Tide gate interlock");
    DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(9))
        .await
        .expect("save and queue");
    drain(&connection, &backend, &blobs.store, &account).await;
    assert_eq!(
        exists(&backend, "Drafts").await,
        2,
        "the server now holds both, which is what the pass below will fetch"
    );

    let mailbox = MailboxRepository::new(&connection)
        .get(drafts_mailbox)
        .await
        .expect("a read")
        .expect("the Drafts folder");
    postio_sync::sync_mailbox(
        &connection,
        &backend,
        &mailbox,
        &postio_account::cancel::CancelToken::new(),
        |_progress: postio_sync::Progress| {},
    )
    .await
    .expect("a sync pass over Drafts");

    let mut after_sync = listed(&connection, drafts_mailbox).await;
    after_sync.sort();
    assert_eq!(
        after_sync,
        vec![
            "Tide gate interlock".to_owned(),
            "Written on the phone".to_owned()
        ],
        "#166: the composer's own draft is already listed, from the row it \
         was given at save time — sync must not remove it"
    );
    assert_eq!(
        after_sync
            .iter()
            .filter(|subject| *subject == "Tide gate interlock")
            .count(),
        1,
        "#51: the sync pass fetched this draft's own append straight back, \
         and must not have added a second row for it"
    );
}
