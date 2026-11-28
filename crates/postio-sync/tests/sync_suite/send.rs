use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{
    Account, Draft, DraftId, EmailAddress, Identity, MailboxId, Operation, OperationTarget,
    TransportSecurity, Uid,
};
use postio_smtp::transport::{ScriptedConnector, SmtpScript};
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, DraftRepository, MailboxRepository, MessageRepository,
    OperationQueueRepository,
};
use postio_storage::test_support;
use postio_sync::Drainer;
use postio_sync::send::SmtpContext;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
}

struct TempBlobs {
    store: BlobStore,
    directory: std::path::PathBuf,
}

impl TempBlobs {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("postio-send-test-{}", uuid_ish()));
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

async fn account_with_sent(connection: &Connection) -> (Account, MailboxId) {
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
        .await
        .expect("create account");
    let sent = test_support::mailbox(connection, &account, "Sent").await;
    (account, sent.id)
}

fn a_draft(account: &Account, to: &str) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.to = vec![EmailAddress::new(None::<String>, to)];
    draft.subject = "Analytical engine".to_owned();
    draft.body.text = Some("Notes on the difference engine.".to_owned());
    draft.start_as(&account.identities[0]);
    draft
}

async fn a_password_source(account: &Account) -> postio_account::auth::StoredPasswordSource {
    let secrets = std::sync::Arc::new(MemorySecretStore::new());
    let key = AccountKey::new(&account.address.address);
    secrets
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("store the password");
    postio_account::auth::StoredPasswordSource::new(secrets)
}

fn accepting_script() -> SmtpScript {
    script_replying_to_rcpt("250 ok")
}

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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent_mailbox) = account_with_sent(&connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
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
            .await
            .expect("get")
            .is_none(),
        "the draft is gone once it is sent"
    );

    let status = backend.status("Sent").await.expect("status");
    assert_eq!(status.exists, 1, "the sent copy landed on the server");

    let local_sent = MailboxRepository::new(&connection)
        .get(sent_mailbox)
        .await
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

    let repository = postio_storage::repository::MessageRepository::new(&connection);
    let status = backend.status("Sent").await.expect("status again");
    let filed = repository
        .by_uid(sent_mailbox, status.generation, Uid::new(1))
        .await
        .expect("look up the filed copy")
        .expect("the filed copy has its wire identity");
    assert_eq!(
        filed
            .server
            .remote_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        Some(format!("{}:{}", status.generation, Uid::new(1))),
        "the filed copy's identity must be the adapter's spelling"
    );

    let page = repository
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent_mailbox),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page of Sent");
    assert_eq!(
        page.len(),
        1,
        "Sent holds {} rows for one sent message: {:?}",
        page.len(),
        page.iter().map(|row| row.id).collect::<Vec<_>>()
    );
    assert_eq!(page[0].id, filed.id, "and it is the row the append named");
}

#[tokio::test]
async fn an_xoauth2_account_sends_with_xoauth2_not_a_password_login() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (mut account, _sent) = account_with_sent(&connection).await;
    account.auth = postio_model::account::AuthMethod::XOAuth2;
    postio_storage::repository::AccountRepository::new(&connection)
        .update(&mut account)
        .await
        .expect("store the mechanism");

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let script = SmtpScript::new("220 mail.example.com ESMTP ready")
        .on(
            "EHLO",
            "250-mail.example.com
250 AUTH XOAUTH2",
        )
        .on("AUTH XOAUTH2", "235 authenticated")
        .on("MAIL FROM", "250 ok")
        .on("RCPT TO", "250 ok")
        .on("DATA", "354 go ahead")
        .on("QUIT", "221 bye");
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 1, "{report:?}");
    let commands = connector.log().commands();
    assert!(
        commands.iter().any(|line| line.starts_with("AUTH XOAUTH2")),
        "the stored mechanism reached the wire: {commands:?}"
    );
}

#[tokio::test]
async fn a_permanent_rejection_fails_without_filing_anything() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent_mailbox) = account_with_sent(&connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let script = script_replying_to_rcpt("550 no such user");
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 0);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].op_type, "send");
    let reason = &report.failed[0].reason;
    assert!(
        reason.contains("550") && reason.contains("no such user"),
        "the failure has to name the server's own answer: {reason}"
    );
    assert!(
        reason.contains("grace@example.net"),
        "and which recipient it was about: {reason}"
    );

    let kept = DraftRepository::new(&connection)
        .get(draft_id)
        .await
        .expect("get")
        .expect("a message never delivered keeps its draft");
    assert_eq!(
        kept.state,
        postio_model::DraftState::Failed,
        "a send the server refused outright left the draft claiming to be on \
         its way, with nothing left to carry it"
    );
    let status = backend.status("Sent").await.expect("status");
    assert_eq!(status.exists, 0, "nothing was ever appended");
    let local_sent = MailboxRepository::new(&connection)
        .get(sent_mailbox)
        .await
        .expect("get")
        .expect("the sent mailbox");
    assert_eq!(local_sent.counts.total, 0);
}

#[tokio::test]
async fn a_transient_rejection_is_deferred_rather_than_failed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent_mailbox) = account_with_sent(&connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let script = script_replying_to_rcpt("450 mailbox temporarily unavailable");
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
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
            .await
            .expect("get")
            .is_some()
    );
}

#[tokio::test]
async fn bcc_recipients_reach_the_envelope_but_never_the_wire_content() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent_mailbox) = account_with_sent(&connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
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

#[tokio::test]
async fn sending_a_draft_takes_its_copy_out_of_the_drafts_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent_mailbox) = account_with_sent(&connection).await;
    test_support::mailbox(&connection, &account, "Drafts").await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(8))
        .await
        .map(|_| draft.id)
        .expect("save and queue the draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue the send");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .mailbox(MockMailbox::new("Drafts"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 2, "{report:?}");
    assert_eq!(
        backend.status("Drafts").await.expect("status").exists,
        0,
        "the draft does not outlive the message it became"
    );
    assert_eq!(backend.status("Sent").await.expect("status").exists, 1);
}

#[derive(Debug)]
struct RotatingSource {
    first: String,
    second: String,
    invalidated: std::sync::atomic::AtomicUsize,
}

impl RotatingSource {
    fn new(first: &str, second: &str) -> Self {
        Self {
            first: first.to_owned(),
            second: second.to_owned(),
            invalidated: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl postio_account::auth::TokenSource for RotatingSource {
    async fn access_token(
        &self,
        _account: &AccountKey,
    ) -> Result<Password, postio_account::secret::SecretError> {
        Ok(Password::new(
            if self.invalidated.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                &self.first
            } else {
                &self.second
            },
        ))
    }

    async fn invalidate(&self, _account: &AccountKey) {
        self.invalidated
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn presented_credentials(connector: &ScriptedConnector) -> Vec<String> {
    use base64::Engine;
    connector
        .log()
        .commands()
        .iter()
        .filter_map(|line| line.strip_prefix("AUTH PLAIN ").map(str::to_owned))
        .filter_map(|argument| {
            base64::engine::general_purpose::STANDARD
                .decode(argument.trim())
                .ok()
        })
        .filter_map(|decoded| String::from_utf8(decoded).ok())
        .map(|sasl| sasl.rsplit('\0').next().unwrap_or_default().to_owned())
        .collect()
}

async fn send_with(
    tokens: &dyn postio_account::auth::TokenSource,
    connector: &ScriptedConnector,
) -> postio_sync::DrainReport {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let blobs = TempBlobs::new();

    drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector,
            tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await
}

#[tokio::test]
async fn a_send_presents_what_the_accounts_token_source_hands_out() {
    let tokens = RotatingSource::new("the-access-token", "unused");
    let connector = ScriptedConnector::new(accepting_script());

    let report = send_with(&tokens, &connector).await;

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(
        presented_credentials(&connector),
        vec!["the-access-token".to_owned()],
        "the credential on the wire is the source's, not the keyring's"
    );
}

#[tokio::test]
async fn a_rejected_send_credential_is_invalidated_once_and_retried_once() {
    let tokens = RotatingSource::new("stale-token", "fresh-token");
    let script = SmtpScript::new("220 mail.example.com ESMTP ready")
        .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "535 5.7.8 authentication failed")
        .on("QUIT", "221 bye");
    let connector = ScriptedConnector::new(script);

    let report = send_with(&tokens, &connector).await;

    assert!(
        report.failed.is_empty() || report.applied == 0,
        "{report:?}"
    );
    assert_eq!(
        presented_credentials(&connector),
        vec!["stale-token".to_owned(), "fresh-token".to_owned()],
        "the stale one, then the fresh one, and never a third"
    );
    assert_eq!(
        tokens.invalidated.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "invalidated once, so the source knew to produce something else"
    );
}

async fn enqueue_send(
    connection: &Connection,
    account: postio_model::AccountId,
    draft_id: DraftId,
) {
    OperationQueueRepository::new(connection)
        .enqueue(
            account,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
}

#[tokio::test]
async fn a_draft_already_accepted_is_never_submitted_again() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_sent(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = drafts.save(&mut draft).await.expect("save draft");
    enqueue_send(&connection, account.id, draft_id).await;
    drafts
        .set_state(draft_id, postio_model::DraftState::Sent)
        .await
        .expect("the drainer's post-acceptance commit");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(
        report.obsolete, 1,
        "an accepted send has nothing left to do: {report:?}"
    );
    assert_eq!(report.applied, 0);
    assert!(report.failed.is_empty(), "{report:?}");

    let log = connector.log();
    assert!(
        log.tcp.is_empty() && log.tls.is_empty(),
        "nothing may reach a submission server for a message it already has: \
         {log:?}"
    );
    assert_eq!(
        backend.status("Sent").await.expect("status").exists,
        0,
        "and no second copy is filed either"
    );
}

#[tokio::test]
async fn a_send_interrupted_mid_submission_is_not_retried_behind_the_users_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_sent(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = drafts.save(&mut draft).await.expect("save draft");
    enqueue_send(&connection, account.id, draft_id).await;
    drafts
        .set_state(draft_id, postio_model::DraftState::Sending)
        .await
        .expect("the mark taken before the transaction opened");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    let log = connector.log();
    assert!(
        log.tcp.is_empty() && log.tls.is_empty(),
        "a message that may already be on its way must not be sent again: {log:?}"
    );
    assert_eq!(report.applied, 0);
    assert_eq!(
        report.deferred, 0,
        "and it is not queued to try later either"
    );
    assert!(
        report.failed.is_empty(),
        "a send that may have gone must not travel as a failure: {report:?}"
    );
    assert_eq!(report.uncertain.len(), 1, "{report:?}");
    let reason = &report.uncertain[0].reason;
    assert!(
        reason.contains("may"),
        "the reason has to leave the question open rather than claim it did \
         not go: {reason}"
    );
    assert_eq!(
        drafts
            .get(draft_id)
            .await
            .expect("read")
            .expect("the draft is still there")
            .state,
        postio_model::DraftState::Unconfirmed,
        "and the durable half says so too -- the queue row is settled and \
         gone, so the draft is the only place this question survives"
    );
}

#[tokio::test]
async fn the_submitted_message_carries_the_reserved_id() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_sent(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).await.expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .await
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("a reservation");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    assert_eq!(report.applied, 1, "{report:?}");

    let written = String::from_utf8_lossy(&connector.log().written).to_lowercase();
    assert!(
        written.contains(&reserved.without_brackets().to_lowercase()),
        "the reserved id has to be in the bytes DATA carried, not merely in \
         the row: looked for {reserved}"
    );
}

#[tokio::test]
async fn a_deferred_send_goes_out_under_the_id_the_first_attempt_reserved() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_sent(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).await.expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .await
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("a reservation");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let blobs = TempBlobs::new();

    let refused = ScriptedConnector::new(script_replying_to_rcpt("450 slow down"));
    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &refused,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    assert_eq!(report.deferred, 1, "{report:?}");
    let after_refusal = drafts
        .get(draft.id)
        .await
        .expect("get")
        .expect("still here");
    assert_eq!(
        after_refusal.rfc_message_id,
        Some(reserved.clone()),
        "a deferral must not spend the reservation",
    );
    assert_eq!(
        after_refusal.state,
        postio_model::DraftState::Queued,
        "the `Sending` mark comes back off when the server answers and \
         refuses: leaving it on would make `resolve` reject the very retry \
         the backoff just scheduled, and a 4xx from a rate-limited server \
         would quietly end the message's life",
    );

    let accepted = ScriptedConnector::new(accepting_script());
    let report = Drainer::new(&backend)
        .with_smtp(SmtpContext {
            connector: &accepted,
            tokens: &tokens,
            blobs: &blobs.store,
        })
        .drain(&connection, account.id, at(11))
        .await
        .expect("drain");
    assert_eq!(report.applied, 1, "{report:?}");

    let written = String::from_utf8_lossy(&accepted.log().written).to_lowercase();
    assert!(
        written.contains(&reserved.without_brackets().to_lowercase()),
        "the retry has to be the same message the first attempt was, or a \
         receiver has no way to tell it is one: looked for {reserved}",
    );
}

async fn drain_a_send_over(
    connection: &Connection,
    connector: &ScriptedConnector,
) -> (postio_sync::DrainReport, postio_model::DraftId) {
    let (account, _sent_mailbox) = account_with_sent(connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let blobs = TempBlobs::new();

    let report = drain_one(
        connection,
        &backend,
        SmtpContext {
            connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    (report, draft_id)
}

#[tokio::test]
async fn a_send_interrupted_once_the_payload_was_on_the_wire_is_not_retried() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");

    let connector = ScriptedConnector::new(accepting_script()).vanishing_after_the_payload();
    let (report, draft_id) = drain_a_send_over(&connection, &connector).await;

    assert_eq!(report.applied, 0);
    assert_eq!(
        report.deferred, 0,
        "a deferral is a retry with a timer on it, which is the one thing \
         this must not be: {report:?}"
    );
    assert!(
        report.failed.is_empty(),
        "a send that may have arrived must not travel as a failure -- the \
         runtime turns `failed` straight into an error the user reads \
         (#674): {report:?}"
    );
    assert_eq!(report.uncertain.len(), 1, "{report:?}");

    let draft = DraftRepository::new(&connection)
        .get(draft_id)
        .await
        .expect("get")
        .expect("a draft that may have been sent is not deleted");
    assert_eq!(
        draft.state,
        postio_model::DraftState::Unconfirmed,
        "the draft carries the question durably: the queue row is settled \
         and gone, and this is where the user finds it again (#674). \
         `resolve` still refuses to submit it on the next process."
    );
    assert!(
        connector.log().written.ends_with(b"\r\n.\r\n"),
        "the fixture is wrong if the payload never went out"
    );
}

#[tokio::test]
async fn a_send_interrupted_before_the_payload_is_queued_again() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");

    let connector = ScriptedConnector::new(accepting_script()).vanishing_at("MAIL FROM");
    let (report, draft_id) = drain_a_send_over(&connection, &connector).await;

    assert_eq!(report.applied, 0);
    assert_eq!(
        report.deferred, 1,
        "a transport failure before the payload is retryable: {report:?}"
    );
    assert!(report.failed.is_empty(), "{report:?}");

    let draft = DraftRepository::new(&connection)
        .get(draft_id)
        .await
        .expect("get")
        .expect("still a draft");
    assert_eq!(
        draft.state,
        postio_model::DraftState::Queued,
        "the `Sending` mark has to come back off, or `resolve` would refuse \
         the retry the backoff is about to schedule"
    );
}

#[tokio::test]
async fn an_unconfirmed_send_resolves_when_its_message_turns_up() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = account_with_sent(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).await.expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .await
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("a reservation");
    drafts
        .set_state(draft.id, postio_model::DraftState::Unconfirmed)
        .await
        .expect("the state an interrupted submission leaves");

    let mut message = postio_model::Message::new(account.id, sent, at(10));
    message.rfc_message_id = Some(reserved.clone());
    let message_id = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("the synced copy");

    let resolved = postio_sync::send::confirm_unconfirmed(&connection, account.id)
        .await
        .expect("confirming reads the store and nothing else");

    assert_eq!(
        resolved,
        vec![(draft.id, message_id)],
        "the draft and the message it turned out to be"
    );
    let after = drafts
        .get(draft.id)
        .await
        .expect("read")
        .expect("still there");
    assert_eq!(
        after.state,
        postio_model::DraftState::Sent,
        "the question is answered, so it stops being unconfirmed"
    );
    assert_eq!(
        drafts
            .by_message(message_id)
            .await
            .expect("read")
            .map(|found| found.id),
        Some(draft.id),
        "and the two are linked, so the surfaces that open one reach the other"
    );
}

#[tokio::test]
async fn an_unconfirmed_send_is_not_resolved_by_a_different_message() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = account_with_sent(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).await.expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .await
        .expect("queue the send");
    drafts
        .set_state(draft.id, postio_model::DraftState::Unconfirmed)
        .await
        .expect("the state an interrupted submission leaves");

    let mut other = postio_model::Message::new(account.id, sent, at(10));
    other.rfc_message_id = Some(postio_model::RfcMessageId::new(
        "<someone-else@example.com>",
    ));
    MessageRepository::new(&connection)
        .create(&mut other)
        .await
        .expect("an unrelated message");

    let resolved = postio_sync::send::confirm_unconfirmed(&connection, account.id)
        .await
        .expect("confirm");

    assert!(
        resolved.is_empty(),
        "only the reserved id is evidence: {resolved:?}"
    );
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("read")
            .expect("still there")
            .state,
        postio_model::DraftState::Unconfirmed,
        "the question is still open, and saying otherwise would be a lie the \
         user acts on"
    );
}

#[tokio::test]
async fn a_send_that_never_reaches_the_server_is_already_in_sent() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent_mailbox) = account_with_sent(&connection).await;

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script()).vanishing_at("EHLO");
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.applied, 0, "nothing was delivered");
    assert!(
        report.failed.is_empty(),
        "a connection lost before any mail command is retryable, not a \
         failure: {:?}",
        report.failed
    );

    let status = backend.status("Sent").await.expect("status");
    assert_eq!(status.exists, 0, "nothing was appended to the server");

    let local_sent = MailboxRepository::new(&connection)
        .get(sent_mailbox)
        .await
        .expect("get")
        .expect("the sent mailbox");
    assert_eq!(
        local_sent.counts.total, 1,
        "the message is not in Sent while it is on its way, which is the \
         'into the ether' this fixes"
    );

    let page = MessageRepository::new(&connection)
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent_mailbox),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page of Sent");
    assert_eq!(page.len(), 1);
    assert!(
        page[0]
            .subject
            .as_deref()
            .is_some_and(|subject| subject.contains("Analytical engine")),
        "the Sent row is not the draft that was sent: {:?}",
        page[0].subject
    );

    assert!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .await
            .expect("get")
            .is_some(),
        "a send still in flight keeps its draft"
    );
}

#[tokio::test]
async fn the_sent_copy_lands_in_the_folder_the_server_actually_has() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, stale_sent) = account_with_sent(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);
    let mut retired = mailboxes
        .get(stale_sent)
        .await
        .expect("get")
        .expect("the stale row");
    retired.selectable = false;
    mailboxes.update(&retired).await.expect("retire it");
    let live = test_support::mailbox(&connection, &account, "Sent Items").await;
    assert_eq!(
        live.role,
        postio_model::MailboxRole::Sent,
        "named for the role"
    );

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent Items"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();
    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    assert_eq!(report.applied, 1, "{report:?}");

    let status = backend.status("Sent Items").await.expect("status");
    assert_eq!(
        status.exists, 1,
        "the sent copy is filed in the Sent folder the server has"
    );
    let counts = async |id| {
        mailboxes
            .get(id)
            .await
            .expect("get")
            .expect("row")
            .counts
            .total
    };
    assert_eq!(
        counts(live.id).await,
        1,
        "and the local row for that folder holds it"
    );
    assert_eq!(
        counts(stale_sent).await,
        0,
        "nothing is filed into a folder the server no longer has"
    );
}

#[tokio::test]
async fn a_reply_joins_its_conversation_locally_before_the_server_is_told() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = account_with_sent(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut parent = postio_model::Message::new(account.id, sent, at(8));
    parent.subject = Some("Analytical engine".to_owned());
    parent.rfc_message_id = Some(postio_model::RfcMessageId::new("parent@example.invalid"));
    let parent_id = messages.create(&mut parent).await.expect("the parent");
    postio_storage::repository::ThreadingRepository::new(&connection, account.id)
        .thread(&messages.get(parent_id).await.expect("get").expect("there"))
        .await
        .expect("thread the parent");
    let parent = messages.get(parent_id).await.expect("get").expect("there");
    let conversation = parent.thread_id.expect("the parent is in a thread");

    let mut draft = a_draft(&account, "grace@example.net");
    draft.in_reply_to = Some(parent_id);
    draft.thread_id = Some(conversation);
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    let filed: Vec<_> = messages
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page of Sent")
        .into_iter()
        .filter(|message| message.id != parent_id)
        .collect();

    let reply = filed
        .first()
        .unwrap_or_else(|| panic!("the reply was never filed locally: {filed:?}"));
    let full = messages.get(reply.id).await.expect("get").expect("there");
    assert_eq!(
        full.mailbox_id, sent,
        "the reply was not filed in Sent, so nothing is visible while it is \
         on its way"
    );
    assert_eq!(
        full.in_reply_to, parent.rfc_message_id,
        "the reply does not name the message it answers, so nothing \
         downstream could thread it even in principle"
    );

    assert_eq!(
        full.thread_id,
        Some(conversation),
        "the reply was filed outside the conversation it answers, so it reads \
         as a new thread and the person watching that conversation sees \
         nothing arrive"
    );

    assert_eq!(
        report.thread_arrivals.into_threads().collect::<Vec<_>>(),
        vec![(conversation, vec![reply.id])],
        "the reply joined its thread and the report does not say so"
    );
}

#[tokio::test]
async fn a_drain_pass_heals_a_send_with_no_operation_behind_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_sent(&connection).await;
    test_support::mailbox(&connection, &account, "Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = drafts.save(&mut draft).await.expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .await
        .expect("queue the send");

    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(10))
        .await
        .expect("the queue")
        .into_iter()
        .find(|op| op.target == OperationTarget::Draft(draft_id))
        .expect("the send is queued");
    OperationQueueRepository::new(&connection)
        .delete(queued.id)
        .await
        .expect("delete the operation");
    assert_eq!(
        drafts
            .get(draft_id)
            .await
            .expect("get")
            .expect("there")
            .state,
        postio_model::DraftState::Queued,
        "the fixture has to start in the state the real store was found in"
    );

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    assert_eq!(
        report.applied, 0,
        "there was nothing to drain; the healing is not an operation: {report:?}"
    );

    assert_eq!(
        drafts
            .get(draft_id)
            .await
            .expect("get")
            .expect("there")
            .state,
        postio_model::DraftState::Failed,
        "running the client left the draft claiming to be on its way, which is \
         the state it cannot get out of by itself"
    );

    let mirror: Option<String> = postio_storage::sql::one(
        &connection,
        "SELECT m.send_state FROM messages m
           JOIN drafts d ON d.message_id = m.id
          WHERE d.id = ?1",
        [draft_id.get()],
        |row| {
            use postio_storage::sql::RowExt as _;
            row.opt_text(0)
        },
    )
    .await
    .expect("the mirror row");
    assert_eq!(
        mirror.as_deref(),
        Some("failed"),
        "the lists read the mirror row, and it still says the send is on its way"
    );
}

const STATEMENT: &[u8] = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
const STATEMENT_BASE64: &str = "JVBERi0xLjQKJeLjz9MK\r\n";

fn a_statement() -> postio_account::backend::MockMessage {
    use postio_account::backend::{BodyStructure, MockMessage, PartNode};
    let structure = BodyStructure::from_parts(
        "multipart/mixed",
        [
            PartNode::new("1", "text/plain", 26)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            PartNode::new("2", "application/pdf", STATEMENT.len() as u64)
                .with_encoding("base64")
                .with_filename("statement.pdf"),
        ],
    );
    MockMessage::new(
        b"From: Grace Hopper <grace@example.net>\r\n\
          To: Ada Lovelace <ada@example.com>\r\n\
          Subject: Statement\r\n\
          Message-ID: <statement@example.net>\r\n\
          Content-Type: multipart/mixed; boundary=b\r\n\
          \r\n\
          --b\r\n\
          Content-Type: text/plain; charset=utf-8\r\n\
          \r\n\
          Your statement is attached.\r\n\
          --b--\r\n"
            .to_vec(),
    )
    .with_internal_date(at(8))
    .with_structure(structure)
    .with_part("1", &b"Your statement is attached."[..])
    .with_part("2", STATEMENT_BASE64.as_bytes())
}

async fn the_statement_synced(
    connection: &postio_storage::Checkout,
    account: &Account,
    backend: &MockBackend,
) -> postio_model::Message {
    let inbox = test_support::mailbox(connection, account, "INBOX").await;
    postio_sync::sync_mailbox(
        connection,
        backend,
        &inbox,
        &postio_account::cancel::CancelToken::new(),
        |_| {},
    )
    .await
    .expect("header sync");
    let messages = MessageRepository::new(connection);
    let uid = *messages
        .uids_in(
            inbox.id,
            backend.status("INBOX").await.expect("status").generation,
        )
        .await
        .expect("uids")
        .first()
        .expect("the statement was synced");
    let source = messages
        .by_uid(
            inbox.id,
            backend.status("INBOX").await.expect("status").generation,
            uid,
        )
        .await
        .expect("look up")
        .expect("the row");
    let pdf = source
        .attachments
        .iter()
        .find(|part| part.part_id.as_deref() == Some("2"))
        .expect("the PDF's metadata came down with the headers");
    assert!(
        pdf.blob_id.is_none(),
        "the fixture is only about a part that was never downloaded"
    );
    source
}

fn submitted(connector: &ScriptedConnector) -> Vec<u8> {
    let written = connector.log().written;
    let text = String::from_utf8_lossy(&written).into_owned();
    let start = text.find("DATA\r\n").expect("a DATA command") + "DATA\r\n".len();
    let end = text[start..]
        .find("\r\n.\r\n")
        .expect("a terminated payload")
        + start;
    text.as_bytes()[start..end].to_vec()
}

async fn queue_a_forward(
    connection: &Connection,
    account: &Account,
    source: &postio_model::Message,
) -> DraftId {
    let mut draft =
        postio_model::reply::forward(source, account, postio_model::reply::plain_forward(source));
    draft.to = vec![EmailAddress::new(None::<String>, "quinn@example.org")];
    let draft_id = DraftRepository::new(connection)
        .save(&mut draft)
        .await
        .expect("save the forward");
    OperationQueueRepository::new(connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
    draft_id
}

#[tokio::test]
async fn forwarding_an_attachment_that_was_never_downloaded_sends_its_bytes() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(a_statement()))
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let source = the_statement_synced(&connection, &account, &backend).await;
    let draft_id = queue_a_forward(&connection, &account, &source).await;

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();
    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert!(
        report.failed.is_empty(),
        "the forward was refused rather than sent: {:?}",
        report.failed
    );
    assert_eq!(report.applied, 1, "{report:?}");
    assert!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .await
            .expect("get")
            .is_none(),
        "the forward was sent, so its draft is gone"
    );

    let parsed = postio_model::mime::parse(&submitted(&connector));
    let carried = parsed
        .parts
        .iter()
        .find(|part| part.attachment.filename.as_deref() == Some("statement.pdf"))
        .unwrap_or_else(|| {
            panic!(
                "the forward went out without the attachment it carried: {:?}",
                parsed
                    .parts
                    .iter()
                    .map(|part| part.attachment.filename.clone())
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        carried.content, STATEMENT,
        "and it carries the file's bytes, fetched from the original"
    );

    let original = MessageRepository::new(&connection)
        .get(source.id)
        .await
        .expect("get")
        .expect("the original");
    assert!(
        original
            .attachments
            .iter()
            .find(|part| part.part_id.as_deref() == Some("2"))
            .is_some_and(|part| part.blob_id.is_some()),
        "the fetched section was not recorded on the message it came from"
    );
}

#[tokio::test]
async fn a_forward_whose_original_is_gone_names_the_file_it_could_not_carry() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(a_statement()))
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let source = the_statement_synced(&connection, &account, &backend).await;
    let draft_id = queue_a_forward(&connection, &account, &source).await;
    MessageRepository::new(&connection)
        .delete(&[source.id])
        .await
        .expect("expunge the original");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();
    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;

    assert_eq!(report.failed.len(), 1, "{report:?}");
    let reason = &report.failed[0].reason;
    assert!(
        reason.contains("statement.pdf"),
        "the refusal must name the file the person will look for: {reason:?}"
    );
    assert!(
        !reason.contains("uploading"),
        "nothing was being uploaded: {reason:?}"
    );
    assert!(
        !connector
            .log()
            .commands()
            .iter()
            .any(|line| line.starts_with("MAIL FROM")),
        "a forward missing its attachment is not submitted without it"
    );
    assert_eq!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .await
            .expect("get")
            .expect("the draft stays for the person to fix")
            .state,
        postio_model::DraftState::Failed
    );
}

async fn sent_to(connection: &Connection, address: &str) -> Option<u32> {
    postio_storage::repository::CorrespondentRepository::new(connection)
        .get(&EmailAddress::new(None::<String>, address))
        .await
        .expect("a read")
        .map(|correspondent| correspondent.sent_count)
}

async fn send_one(
    connection: &Connection,
    account: &Account,
    mut draft: Draft,
    script: SmtpScript,
) -> postio_sync::DrainReport {
    let draft_id = DraftRepository::new(connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(account).await;
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();
    drain_one(
        connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await
}

#[tokio::test]
async fn sending_to_three_addresses_adds_one_to_each_correspondent() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;
    let mut draft = a_draft(&account, "grace@example.net");
    draft
        .to
        .push(EmailAddress::new(Some("Tove"), "tove@example.org"));
    draft.cc = vec![EmailAddress::new(None::<String>, "quinn@example.com")];

    let report = send_one(&connection, &account, draft, accepting_script()).await;
    assert_eq!(report.applied, 1, "{report:?}");

    for to in ["grace@example.net", "tove@example.org", "quinn@example.com"] {
        assert_eq!(sent_to(&connection, to).await, Some(1), "{to}");
    }
    assert_eq!(
        sent_to(&connection, "ada@example.com").await,
        None,
        "the sender is nobody she wrote to"
    );
}

async fn saved(connection: &Connection, address: &str) -> bool {
    postio_storage::repository::ContactRepository::new(connection)
        .by_address(address)
        .await
        .expect("a read")
        .is_some_and(|person| person.is_saved())
}

async fn turn_saving_on(connection: &Connection, on: bool) {
    postio_storage::repository::ContactSuggestionRepository::new(connection)
        .set(postio_model::Suggestion::SaveAutomatically, on)
        .await
        .expect("the switch");
}

#[tokio::test]
async fn with_save_people_i_write_to_on_a_send_saves_to_cc_and_bcc_at_once() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;
    turn_saving_on(&connection, true).await;
    let mut draft = a_draft(&account, "grace@example.net");
    draft.cc = vec![EmailAddress::new(None::<String>, "quinn@example.com")];
    draft.bcc = vec![EmailAddress::new(None::<String>, "tove@example.org")];

    let report = send_one(&connection, &account, draft, accepting_script()).await;
    assert_eq!(report.applied, 1, "{report:?}");

    for who in ["grace@example.net", "quinn@example.com", "tove@example.org"] {
        assert!(saved(&connection, who).await, "{who} is saved by the send");
    }
}

#[tokio::test]
async fn with_it_off_a_send_saves_nobody() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;
    turn_saving_on(&connection, false).await;

    let report = send_one(
        &connection,
        &account,
        a_draft(&account, "grace@example.net"),
        accepting_script(),
    )
    .await;
    assert_eq!(report.applied, 1, "{report:?}");
    assert!(!saved(&connection, "grace@example.net").await);
}

#[tokio::test]
async fn a_refused_send_saves_nobody_even_with_the_switch_on() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;
    turn_saving_on(&connection, true).await;
    let report = send_one(
        &connection,
        &account,
        a_draft(&account, "grace@example.net"),
        script_replying_to_rcpt("550 no such user"),
    )
    .await;
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert!(!saved(&connection, "grace@example.net").await);
}

#[tokio::test]
async fn a_send_the_server_refused_wrote_to_nobody() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;

    let report = send_one(
        &connection,
        &account,
        a_draft(&account, "grace@example.net"),
        script_replying_to_rcpt("550 no such user"),
    )
    .await;
    assert_eq!(report.failed.len(), 1, "{report:?}");

    assert!(
        !postio_storage::repository::CorrespondentRepository::new(&connection)
            .wrote_to(&EmailAddress::new(None::<String>, "grace@example.net"))
            .await
            .expect("a read"),
        "nothing was delivered, so nobody was written to"
    );
}

#[tokio::test]
async fn labels_chosen_before_sending_are_carried_by_the_conversation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = account_with_sent(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX")
        .await
        .id;
    let messages = MessageRepository::new(&connection);
    let labels = postio_storage::repository::LabelRepository::new(&connection);
    let mut harbour = postio_model::Label::new(account.id, "Harbour");
    let mut travel = postio_model::Label::new(account.id, "Travel");
    labels.create(&mut harbour).await.expect("a label");
    labels.create(&mut travel).await.expect("a label");

    let mut parent = postio_model::Message::new(account.id, inbox, at(8));
    parent.subject = Some("Harbour plan".to_owned());
    parent.rfc_message_id = Some(postio_model::RfcMessageId::new("plan@example.invalid"));
    parent.from = vec![EmailAddress::new(None::<String>, "grace@example.net")];
    parent.labels = vec![harbour.id];
    parent.flags = [
        postio_model::Flag::Seen,
        postio_model::Flag::Keyword("Harbour".to_owned()),
    ]
    .into_iter()
    .collect();
    let parent_id = messages.create(&mut parent).await.expect("the parent");
    postio_storage::repository::ThreadingRepository::new(&connection, account.id)
        .thread(&messages.get(parent_id).await.expect("get").expect("there"))
        .await
        .expect("thread the parent");
    let conversation = messages
        .get(parent_id)
        .await
        .expect("get")
        .expect("there")
        .thread_id
        .expect("the parent is in a thread");

    let mut draft = a_draft(&account, "grace@example.net");
    draft.in_reply_to = Some(parent_id);
    draft.thread_id = Some(conversation);
    draft.labels = vec![harbour.id, travel.id];
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();
    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    assert_eq!(report.applied, 1, "{report:?}");

    let thread = postio_storage::repository::ThreadRepository::new(&connection)
        .get(conversation)
        .await
        .expect("a read")
        .expect("the conversation");
    assert_eq!(thread.labels, vec![harbour.id, travel.id]);

    let copy = messages
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page of Sent")
        .first()
        .map(|row| row.id)
        .expect("the reply was filed in Sent");
    let copy = messages.get(copy).await.expect("get").expect("there");
    assert_eq!(copy.thread_id, Some(conversation));
    assert_eq!(copy.labels, vec![harbour.id, travel.id]);
    for name in ["Harbour", "Travel"] {
        assert!(
            copy.flags
                .contains(&postio_model::Flag::Keyword(name.to_owned())),
            "the sent copy's {name} keyword: {:?}",
            copy.flags
        );
    }

    let parent = messages.get(parent_id).await.expect("get").expect("there");
    assert_eq!(parent.labels, vec![harbour.id, travel.id]);
    assert!(
        parent
            .flags
            .contains(&postio_model::Flag::Keyword("Travel".to_owned()))
    );
    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(12))
        .await
        .expect("the queue");
    assert!(
        queued.iter().any(|operation| {
            operation.target == OperationTarget::Message(parent_id)
                && matches!(
                    &operation.operation,
                    Operation::SetFlags { flags }
                        if flags.contains(&postio_model::Flag::Keyword("Travel".to_owned()))
                )
        }),
        "the server is told, as for any label: {queued:?}"
    );

    let on_server = backend
        .fetch_headers(
            "Sent",
            &postio_account::backend::UidSet::all(),
            None,
            &postio_account::cancel::CancelToken::new(),
        )
        .await
        .expect("the server's Sent");
    assert_eq!(on_server.len(), 1);
    for name in ["Harbour", "Travel"] {
        assert!(
            on_server[0]
                .flags
                .contains(&postio_model::Flag::Keyword(name.to_owned())),
            "appended with its {name} keyword: {:?}",
            on_server[0].flags
        );
    }
}

#[tokio::test]
async fn a_draft_s_remind_if_no_reply_becomes_a_reminder_on_its_conversation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = account_with_sent(&connection).await;
    let due = at(9) + chrono::Duration::days(3);
    let mut draft = a_draft(&account, "grace@example.net");
    draft.remind_at = Some(due);
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();
    let report = drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    assert_eq!(report.applied, 1, "{report:?}");

    let messages = MessageRepository::new(&connection);
    let copy = messages
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page of Sent")
        .first()
        .map(|row| row.id)
        .expect("the message was filed in Sent");
    let copy = messages.get(copy).await.expect("get").expect("there");
    let conversation = copy.thread_id.expect("the sent copy is in a thread");
    let reminder = postio_storage::repository::ReminderRepository::new(&connection)
        .standing(conversation)
        .await
        .expect("a read")
        .expect("sending set a reminder on the conversation");
    assert_eq!(reminder.due_at, due, "due when the composer said");
    assert_eq!(
        reminder.anchor, copy.id,
        "waiting on a reply to what was sent"
    );
    assert_eq!(reminder.fired_at, None, "not fired yet");
}

#[tokio::test]
async fn a_draft_with_no_remind_if_no_reply_sets_no_reminder() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = account_with_sent(&connection).await;
    let mut draft = a_draft(&account, "grace@example.net");
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();
    drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    let messages = MessageRepository::new(&connection);
    let copy = messages
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page of Sent")
        .first()
        .map(|row| row.id)
        .expect("the message was filed in Sent");
    let conversation = messages
        .get(copy)
        .await
        .expect("get")
        .expect("there")
        .thread_id
        .expect("in a thread");
    assert_eq!(
        postio_storage::repository::ReminderRepository::new(&connection)
            .standing(conversation)
            .await
            .expect("a read"),
        None
    );
}

async fn reply_to_a_parent(
    script: SmtpScript,
) -> (
    Vec<postio_storage::repository::QueuedOperation>,
    postio_model::Message,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _sent) = account_with_sent(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX")
        .await
        .id;
    let messages = MessageRepository::new(&connection);
    let mut parent = postio_model::Message::new(account.id, inbox, at(8));
    parent.subject = Some("Harbour plan".to_owned());
    parent.rfc_message_id = Some(postio_model::RfcMessageId::new("plan@example.invalid"));
    parent.from = vec![EmailAddress::new(None::<String>, "grace@example.net")];
    parent.flags = [postio_model::Flag::Seen].into_iter().collect();
    let parent_id = messages.create(&mut parent).await.expect("the parent");

    let mut draft = a_draft(&account, "grace@example.net");
    draft.in_reply_to = Some(parent_id);
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let draft_id = DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .await
        .expect("enqueue");
    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(script);
    let blobs = TempBlobs::new();
    drain_one(
        &connection,
        &backend,
        SmtpContext {
            connector: &connector,
            tokens: &tokens,
            blobs: &blobs.store,
        },
        account.id,
    )
    .await;
    let queued = OperationQueueRepository::new(&connection)
        .pending(account.id, at(12))
        .await
        .expect("the queue");
    let parent = messages.get(parent_id).await.expect("get").expect("there");
    (queued, parent)
}

fn tells_the_server_it_was_answered(
    queued: &[postio_storage::repository::QueuedOperation],
    parent: &postio_model::Message,
) -> bool {
    queued.iter().any(|operation| {
        operation.target == OperationTarget::Message(parent.id)
            && matches!(
                &operation.operation,
                Operation::SetFlags { flags } if flags.contains(&postio_model::Flag::Answered)
            )
    })
}

#[tokio::test]
async fn a_delivered_reply_marks_what_it_answers_and_tells_the_server() {
    let (queued, parent) = reply_to_a_parent(accepting_script()).await;
    assert!(
        parent.flags.contains(&postio_model::Flag::Answered),
        "every other client shows the replied arrow from \\Answered: {:?}",
        parent.flags
    );
    assert!(
        parent.flags.contains(&postio_model::Flag::Seen),
        "the mark is added, never a replacement for the flags it had"
    );
    assert!(
        tells_the_server_it_was_answered(&queued, &parent),
        "the server is told like any flag: {queued:?}"
    );
}

#[tokio::test]
async fn a_refused_reply_leaves_what_it_answers_unmarked() {
    let (queued, parent) = reply_to_a_parent(script_replying_to_rcpt("550 no such user")).await;
    assert!(
        !parent.flags.contains(&postio_model::Flag::Answered),
        "nothing was delivered, so nothing was answered"
    );
    assert!(!tells_the_server_it_was_answered(&queued, &parent));
}
