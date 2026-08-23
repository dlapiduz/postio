//! Sending a `Draft` through the operation queue, end to end: SMTP over a
//! scripted transcript, no network and no server, mirroring how the rest of
//! this crate is developed against `MockBackend`.

use chrono::{DateTime, TimeZone, Utc};
use postio_imap::backend::{MailBackend, MockBackend, MockMailbox};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{
    Account, Draft, EmailAddress, Identity, MailboxId, Operation, OperationTarget,
    TransportSecurity,
};
use postio_smtp::transport::{ScriptedConnector, SmtpScript};
use postio_storage::BlobStore;
use postio_storage::repository::{
    AccountRepository, DraftRepository, MailboxRepository, OperationQueueRepository,
};
use postio_storage::test_support;
use postio_sync::Drainer;
use postio_sync::send::SmtpContext;
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
        let directory = std::env::temp_dir().join(format!("postio-send-test-{}", uuid_ish()));
        let store = BlobStore::open(&directory).expect("a blob store");
        Self { store, directory }
    }
}

impl Drop for TempBlobs {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

fn uuid_ish() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

/// An account with one identity and a Sent mailbox, ready to send from.
fn account_with_sent(connection: &Connection) -> (Account, MailboxId) {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    account.outgoing.host = "smtp.example.com".to_owned();
    account.outgoing.port = 465;
    account.outgoing.security = TransportSecurity::Tls;
    account.outgoing.username = "ada@example.com".to_owned();

    let mut identity = Identity::new(
        postio_model::ids::AccountId::UNASSIGNED,
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    );
    identity.is_default = true;
    account.identities = vec![identity];

    AccountRepository::new(connection)
        .create(&mut account)
        .expect("create account");
    let sent = test_support::mailbox(connection, &account, "Sent");
    (account, sent.id)
}

/// A draft ready to send, addressed to `to`.
fn a_draft(account: &Account, to: &str) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.to = vec![EmailAddress::new(None::<String>, to)];
    draft.subject = "Analytical engine".to_owned();
    draft.body.text = Some("Notes on the difference engine.".to_owned());
    draft.use_identity(&account.identities[0]);
    draft
}

async fn store_password(secrets: &MemorySecretStore, account: &Account) {
    let key = AccountKey::new(&account.address.address);
    secrets
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("store the password");
}

/// A transcript that accepts everything: `EHLO`, `AUTH PLAIN`, the mail
/// transaction, and `QUIT`.
fn accepting_script() -> SmtpScript {
    script_replying_to_rcpt("250 ok")
}

/// A transcript identical to [`accepting_script`], except `RCPT TO` gets
/// `reply` instead of an accept.
///
/// [`SmtpScript::on`]'s rules match in the order they were added, so this
/// builds the `RCPT TO` rule in rather than appending an override after
/// [`accepting_script`]'s own accept, which would never be reached.
fn script_replying_to_rcpt(reply: &str) -> SmtpScript {
    SmtpScript::new("220 mail.example.com ESMTP ready")
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "235 authenticated")
        .on("MAIL FROM", "250 ok")
        .on("RCPT TO", reply)
        .on("DATA", "354 go ahead")
        .on("QUIT", "221 bye")
}

async fn drain_one(
    connection: &Connection,
    backend: &MockBackend,
    smtp: SmtpContext<'_>,
    account: postio_model::AccountId,
) -> postio_sync::DrainReport {
    Drainer::new(backend)
        .with_smtp(smtp)
        .drain(connection, account, at(10))
        .await
        .expect("drain")
}

#[tokio::test]
async fn sending_a_draft_delivers_it_and_files_a_sent_copy() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, sent_mailbox) = account_with_sent(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let secrets = MemorySecretStore::new();
    store_password(&secrets, &account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            secrets: &secrets,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 1, "{report:?}");
    assert!(report.failed.is_empty());

    assert!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .expect("get")
            .is_none(),
        "the draft is gone once it is sent"
    );

    let status = backend.status("Sent").await.expect("status");
    assert_eq!(status.exists, 1, "the sent copy landed on the server");

    let local_sent = MailboxRepository::new(&connection)
        .get(sent_mailbox)
        .expect("get")
        .expect("the sent mailbox");
    assert_eq!(
        local_sent.counts.total, 1,
        "and a local row exists for it too"
    );

    let commands = connector.log().commands();
    assert!(commands.iter().any(|line| line.starts_with("MAIL FROM")));
    assert!(
        commands
            .iter()
            .any(|line| line.contains("grace@example.net"))
    );
}

#[tokio::test]
async fn a_permanent_rejection_fails_without_filing_anything() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, sent_mailbox) = account_with_sent(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let secrets = MemorySecretStore::new();
    store_password(&secrets, &account).await;
    let script = script_replying_to_rcpt("550 no such user");
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            secrets: &secrets,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 0);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].op_type, "send");

    assert!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .expect("get")
            .is_some(),
        "a message never delivered keeps its draft"
    );
    let status = backend.status("Sent").await.expect("status");
    assert_eq!(status.exists, 0, "nothing was ever appended");
    let local_sent = MailboxRepository::new(&connection)
        .get(sent_mailbox)
        .expect("get")
        .expect("the sent mailbox");
    assert_eq!(local_sent.counts.total, 0);
}

#[tokio::test]
async fn a_transient_rejection_is_deferred_rather_than_failed() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _sent_mailbox) = account_with_sent(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let secrets = MemorySecretStore::new();
    store_password(&secrets, &account).await;
    let script = script_replying_to_rcpt("450 mailbox temporarily unavailable");
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            secrets: &secrets,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 0);
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(report.deferred, 1);
    assert!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .expect("get")
            .is_some()
    );
}

#[tokio::test]
async fn bcc_recipients_reach_the_envelope_but_never_the_wire_content() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _sent_mailbox) = account_with_sent(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let secrets = MemorySecretStore::new();
    store_password(&secrets, &account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            secrets: &secrets,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 1, "{report:?}");

    let log = connector.log();
    assert!(
        log.commands()
            .iter()
            .any(|line| line.contains("quiet@example.com")),
        "the bcc'd address is still a RCPT TO in the envelope"
    );
    let written = String::from_utf8_lossy(&log.written);
    assert!(
        !written.contains("Bcc:") && !written.contains("Bcc "),
        "but it must never appear in the message content itself"
    );
}
