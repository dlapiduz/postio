//! Keeping the Drafts mailbox in step, end to end against `MockBackend`.
//!
//! The composer's half — writing the row and enqueueing — is
//! `postio-storage`'s `tests/drafts.rs`. This is the drain half: what the
//! server ends up holding after the queue goes out.

use chrono::{DateTime, TimeZone, Utc};
use postio_imap::backend::{MailBackend, MockBackend, MockMailbox};
use postio_model::{
    Account, Draft, EmailAddress, Identity, MailboxId, Operation, OperationTarget, Uid, UidValidity,
};
use postio_storage::BlobStore;
use postio_storage::repository::{
    AccountRepository, DraftRepository, MailboxRepository, OperationQueueRepository,
};
use postio_storage::test_support;
use postio_sync::{DrainReport, Drainer};
use rusqlite::Connection;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
}

/// A blob store in a scratch directory that removes itself on drop.
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
        let store = BlobStore::open(&directory).expect("a blob store");
        Self { store, directory }
    }
}

impl Drop for TempBlobs {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

/// An account with one identity and the Drafts mailbox a draft is filed into.
fn account_with_drafts(connection: &Connection) -> (Account, MailboxId) {
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
        .expect("create account");
    let drafts = test_support::mailbox(connection, &account, "Drafts");
    (account, drafts.id)
}

fn a_draft(account: &Account, subject: &str) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.to = vec![EmailAddress::new(None::<String>, "grace@example.net")];
    draft.subject = subject.to_owned();
    draft.body.text = Some("Half a thought, still being had.".to_owned());
    draft.use_identity(&account.identities[0]);
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;

    let mut draft = a_draft(&account, "Tide gate interlock");
    DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(9))
        .expect("save and queue");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(exists(&backend, "Drafts").await, 1);

    let stored = DraftRepository::new(&connection)
        .get(draft.id)
        .expect("get")
        .expect("the draft");
    assert!(
        stored.server.uid.is_some() && stored.server.uid_validity.is_some(),
        "the row learns where its copy landed, which is what lets the next \
         save replace it rather than add to it"
    );
}

#[tokio::test]
async fn editing_a_draft_replaces_its_copy_rather_than_adding_one() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    drafts.save_and_sync(&mut draft, at(9)).expect("save");
    drain(&connection, &backend, &blobs.store, &account).await;
    let first = drafts
        .get(draft.id)
        .expect("get")
        .expect("the draft")
        .server
        .uid;

    draft.body.text = Some("Half a thought, now most of one.".to_owned());
    drafts
        .save_and_sync(&mut draft, at(10))
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
        .expect("get")
        .expect("the draft")
        .server
        .uid;
    assert_ne!(first, second, "and the row follows the copy that is there");
}

#[tokio::test]
async fn a_run_of_autosaves_costs_one_round_trip() {
    // The reason saves fold at all: a minute of typing leaves a queue full of
    // rows that all say the same thing, and the text is not read until the
    // step drains.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    for (index, hour) in [7, 8, 9].into_iter().enumerate() {
        draft.subject = format!("Tide gate interlock, revision {index}");
        drafts.save_and_sync(&mut draft, at(hour)).expect("save");
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    drafts.save_and_sync(&mut draft, at(9)).expect("save");
    drain(&connection, &backend, &blobs.store, &account).await;
    assert_eq!(exists(&backend, "Drafts").await, 1);

    drafts.discard(draft.id, at(10)).expect("discard");
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "Tide gate interlock");
    drafts.save_and_sync(&mut draft, at(9)).expect("save");
    // Discarded while the save is still in the queue: the row goes, so the
    // save has nothing left to upload.
    drafts.discard(draft.id, at(10)).expect("discard");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.obsolete, 1, "{report:?}");
    assert_eq!(report.applied, 0, "{report:?}");
    assert_eq!(exists(&backend, "Drafts").await, 0);
}

#[tokio::test]
async fn a_renumbered_drafts_mailbox_is_never_expunged_by_a_stale_uid() {
    // The hazard the operation carries its `UidValidity` for: under a new
    // generation, UID 1 is somebody else's message.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;

    let mut folder = MailboxRepository::new(&connection)
        .get(drafts_mailbox)
        .expect("get")
        .expect("the drafts mailbox");
    folder.uid_validity = Some(UidValidity::new(2));
    MailboxRepository::new(&connection)
        .update(&folder)
        .expect("record the new generation");

    let mut draft = a_draft(&account, "Tide gate interlock");
    DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft.id),
            &Operation::DiscardDraft {
                mailbox: drafts_mailbox,
                uid: Uid::new(1),
                uid_validity: UidValidity::new(1),
            },
            at(9),
        )
        .expect("enqueue");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.obsolete, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
}

#[tokio::test]
async fn a_draft_whose_attachment_is_still_being_written_waits_rather_than_fails() {
    // The composer writes a dropped file into the blob store off the main
    // thread, so an autosave can reach the queue first. Uploading the draft
    // now would put a copy on the server that is quietly missing part of
    // itself.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
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
        .expect("save");

    let report = drain(&connection, &backend, &blobs.store, &account).await;

    assert_eq!(report.deferred, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(exists(&backend, "Drafts").await, 0);
}

#[tokio::test]
async fn the_copy_in_drafts_keeps_the_bcc_the_sent_message_will_not() {
    // The Drafts folder is the user's own and reaches nobody. Losing Bcc
    // there loses recipients they typed; carrying it in the *sent* bytes
    // would hand every other recipient the list. Both matter, and they are
    // different bytes.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let blobs = TempBlobs::new();
    let backend = a_server("Drafts").await;

    let mut draft = a_draft(&account, "Tide gate interlock");
    draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];
    DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(9))
        .expect("save and queue");

    drain(&connection, &backend, &blobs.store, &account).await;

    let stored = DraftRepository::new(&connection)
        .get(draft.id)
        .expect("get")
        .expect("the draft");
    let uid = stored.server.uid.expect("the copy landed");

    let mut sink = postio_imap::backend::VecSink::new();
    backend
        .fetch_body(
            "Drafts",
            uid,
            &mut sink,
            &postio_imap::cancel::CancelToken::new(),
        )
        .await
        .expect("read the copy back");

    let raw = String::from_utf8_lossy(sink.as_slice()).into_owned();
    assert!(
        raw.contains("quiet@example.com"),
        "the draft on the server has to carry every recipient the user typed"
    );
}
