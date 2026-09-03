//! Sending a `Draft` through the operation queue, end to end: SMTP over a
//! scripted transcript, no network and no server, mirroring how the rest of
//! this crate is developed against `MockBackend`.

use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{
    Account, Draft, DraftId, EmailAddress, Identity, MailboxId, Operation, OperationTarget,
    TransportSecurity, Uid,
};
use postio_smtp::transport::{ScriptedConnector, SmtpScript};
use postio_storage::BlobStore;
use postio_storage::repository::{
    AccountRepository, DraftRepository, MailboxRepository, MessageRepository,
    OperationQueueRepository,
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

/// The password world behind the [`TokenSource`] seam, which is what a
/// password account's composition root builds.
///
/// [`TokenSource`]: postio_account::auth::TokenSource
async fn a_password_source(account: &Account) -> postio_account::auth::StoredPasswordSource {
    let secrets = std::sync::Arc::new(MemorySecretStore::new());
    let key = AccountKey::new(&account.address.address);
    secrets
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("store the password");
    postio_account::auth::StoredPasswordSource::new(secrets)
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

    // #543: a row born from an append carries the backend-neutral identity
    // the adapter spelled for it, agreeing with the wire pair on the row.
    let repository = postio_storage::repository::MessageRepository::new(&connection);
    let status = backend.status("Sent").await.expect("status again");
    let filed = repository
        .by_uid(sent_mailbox, status.generation, Uid::new(1))
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
}

#[tokio::test]
async fn an_xoauth2_account_sends_with_xoauth2_not_a_password_login() {
    // #533: the account's stored mechanism was dropped on the way into the
    // SMTP settings, so every send spoke AUTH PLAIN however the account
    // was stored. The script here answers *only* XOAUTH2 — a send that
    // still says PLAIN has nothing to match and fails, which is exactly
    // the regression this guards.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (mut account, _sent) = account_with_sent(&connection);
    account.auth = postio_model::account::AuthMethod::XOAuth2;
    postio_storage::repository::AccountRepository::new(&connection)
        .update(&mut account)
        .expect("store the mechanism");

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
    // And it says what the *server* said (#674). Once the indeterminate case
    // has its own outcome, everything left in `failed` is on the safe side
    // of the boundary -- so the copy may say the message did not go, and it
    // owes the user the reason rather than "something went wrong". A code
    // and a sentence they can act on, or take to their provider.
    let reason = &report.failed[0].reason;
    assert!(
        reason.contains("550") && reason.contains("no such user"),
        "the failure has to name the server's own answer: {reason}"
    );
    assert!(
        reason.contains("grace@example.net"),
        "and which recipient it was about: {reason}"
    );

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
    // A sent message still showing as an unfinished draft on the user's phone
    // is the same bug as never having uploaded it: the two folders disagree
    // about what happened.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _sent_mailbox) = account_with_sent(&connection);
    test_support::mailbox(&connection, &account, "Drafts");

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(&connection)
        .save_and_sync(&mut draft, at(8))
        .map(|_| draft.id)
        .expect("save and queue the draft");
    OperationQueueRepository::new(&connection)
        .enqueue(
            account.id,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .expect("enqueue the send");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .mailbox(MockMailbox::new("Drafts"))
        .build();
    backend.connect().await.expect("connect");

    let tokens = a_password_source(&account).await;
    let connector = ScriptedConnector::new(accepting_script());
    let blobs = TempBlobs::new();

    // Both rows in one pass: the draft is uploaded, then sent, in the order
    // the user did them.
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

// ---------------------------------------------------------------------------
// The account's TokenSource, shared with IMAP — ADR 0006 Q5, #194
// ---------------------------------------------------------------------------

/// Hands out `first` until invalidated, then `second`, counting both.
///
/// The same shape `postio-account`'s pool tests use, because it is the same
/// discipline being asserted on the other side of the account.
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

/// Every `AUTH PLAIN` argument the transcript saw, decoded.
///
/// SASL PLAIN is `\0user\0secret` in base64, so this is what the server was
/// actually told — the only place "which credential was presented" can
/// honestly be read.
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

/// Queues one send and drains it, so the tests below differ only in the
/// credential and the transcript.
async fn send_with(
    tokens: &dyn postio_account::auth::TokenSource,
    connector: &ScriptedConnector,
) -> postio_sync::DrainReport {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _sent) = account_with_sent(&connection);

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
    // The other half of "one `TokenSource` per account". Sending used to read
    // the keyring directly, under the account's own key -- which for an OAuth
    // account holds no password at all, so the send presented nothing usable
    // however healthy the IMAP side was.
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
    // The same discipline the IMAP pool keeps, at the other place a
    // credential meets a server: an access token that expired between the
    // last IMAP command and this send is the ordinary case, not the edge.
    //
    // The transcript refuses every `AUTH`, so what is asserted is the shape
    // of the attempt rather than its luck -- two credentials presented, the
    // second one different from the first, and then a stop. A source that
    // kept being asked would show a third.
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

// ---------------------------------------------------------------------------
// The commit point (ADR 0021, decision 2)
// ---------------------------------------------------------------------------

/// Queues a `Send` for `draft_id` without going through `queue_send`, so a
/// test can put the draft in whatever state it wants to drain from.
fn enqueue_send(connection: &Connection, account: postio_model::AccountId, draft_id: DraftId) {
    OperationQueueRepository::new(connection)
        .enqueue(
            account,
            OperationTarget::Draft(draft_id),
            &Operation::Send { draft: draft_id },
            at(9),
        )
        .expect("enqueue");
}

/// The crash case, from the side that decides it.
///
/// Before ADR 0021 the durable fact that stopped a resend was the *deletion
/// of the draft row*, which is the second-to-last thing `file_sent_copy`
/// does — behind `QUIT`, a whole `APPEND` of the message to the Sent
/// mailbox, a blob write and three more writes. A process that died anywhere
/// in that window came back to a draft that still existed and a `Send` still
/// pending, and submitted the message a second time.
///
/// Now the draft is committed `Sent` the instant SMTP accepts, before any of
/// that, so the drain that comes after the crash has something to read.
#[tokio::test]
async fn a_draft_already_accepted_is_never_submitted_again() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_sent(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = drafts.save(&mut draft).expect("save draft");
    enqueue_send(&connection, account.id, draft_id);
    // Exactly the state a crash between SMTP accepting and the `APPEND`
    // leaves behind: the mark is committed, the filing never finished, and
    // the row and its queued operation are both still here.
    drafts
        .set_state(draft_id, postio_model::DraftState::Sent)
        .expect("the drainer's post-acceptance commit");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    // Would deliver, if anything asked it to. That is the point: the
    // assertion below is about what did *not* happen, so the transport has
    // to be one that would have succeeded.
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

/// The other half of the same window: a process that died with the SMTP
/// transaction open.
///
/// Nothing can know whether the payload reached the server, so this must not
/// resend — every ambiguity in this path resolves toward asking rather than
/// duplicating. It settles loudly instead, and the wording deliberately does
/// not claim the message failed to go.
///
/// The outcome is `uncertain` rather than `failed` (#674): "failed" means
/// *did not happen*, and the runtime turns that straight into an error the
/// user reads. What has not changed, and must not, is that no second
/// submission happens.
#[tokio::test]
async fn a_send_interrupted_mid_submission_is_not_retried_behind_the_users_back() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_sent(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = drafts.save(&mut draft).expect("save draft");
    enqueue_send(&connection, account.id, draft_id);
    drafts
        .set_state(draft_id, postio_model::DraftState::Sending)
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
            .expect("read")
            .expect("the draft is still there")
            .state,
        postio_model::DraftState::Unconfirmed,
        "and the durable half says so too -- the queue row is settled and \
         gone, so the draft is the only place this question survives"
    );
}

/// The reservation is what makes a retry recognisable downstream, so it has
/// to reach the bytes that are actually submitted.
#[tokio::test]
async fn the_submitted_message_carries_the_reserved_id() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_sent(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
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

/// The headline guarantee, end to end: a send that is deferred and then goes
/// through submits **one** message, not two that happen to say the same
/// thing.
///
/// Before ADR 0021 this was the ordinary case rather than an edge one. A
/// `Send` is resolved — and therefore rebuilt — on every drain attempt, and
/// every build minted a fresh `Message-ID`, so the second attempt was a
/// distinct message no receiver could recognise as a duplicate of the first.
/// A 4xx from a rate-limited server is enough to reach it.
#[tokio::test]
async fn a_deferred_send_goes_out_under_the_id_the_first_attempt_reserved() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_sent(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("a reservation");

    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("Sent"))
        .build();
    backend.connect().await.expect("connect");
    let tokens = a_password_source(&account).await;
    let blobs = TempBlobs::new();

    // Attempt one: the server asks to try later.
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
    let after_refusal = drafts.get(draft.id).expect("get").expect("still here");
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

    // Attempt two, once the backoff has elapsed: it goes.
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

// ---------------------------------------------------------------------------
// The payload boundary reaches the drainer — ADR 0021, #673
// ---------------------------------------------------------------------------

/// Queues one draft and drains it over `connector`, handing back the report,
/// the draft id and the connection so a test can ask what became of both.
///
/// The two tests below differ in exactly one thing — where the connection
/// dies — so everything else is shared, and the difference is legible.
async fn drain_a_send_over(
    connection: &Connection,
    connector: &ScriptedConnector,
) -> (postio_sync::DrainReport, postio_model::DraftId) {
    let (account, _sent_mailbox) = account_with_sent(connection);

    let mut draft = a_draft(&account, "grace@example.net");
    let draft_id = DraftRepository::new(connection)
        .save(&mut draft)
        .expect("save draft");
    OperationQueueRepository::new(connection)
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
    // ADR 0021's whole point, driven for the first time through a transport
    // rather than asserted on a hand-built error: the payload went out, the
    // server never answered, and the drainer must not try again. "Try again"
    // and "you already sent it" are indistinguishable from here, and a second
    // copy in somebody else's inbox cannot be recalled.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");

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
    // The other half, and why narrowing the predicate matters: nothing of the
    // message reached the server, so this is an ordinary connection hiccup.
    // Refusing to retry it would strand a queued message that was never sent.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");

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
        .expect("get")
        .expect("still a draft");
    assert_eq!(
        draft.state,
        postio_model::DraftState::Queued,
        "the `Sending` mark has to come back off, or `resolve` would refuse \
         the retry the backoff is about to schedule"
    );
}

/// A sync that brings the message down is the confirmation, and it costs the
/// user nothing.
///
/// ADR 0021 Decision 3's second half (#674): the draft reserved a
/// `Message-ID` before the first attempt (#461), so a message arriving with
/// that id in this account is proof the submission went through — whatever
/// the dropped connection left the client believing. Many submission servers
/// file the sender's copy themselves, so for a good share of users this
/// resolves silently within one sync.
#[tokio::test]
async fn an_unconfirmed_send_resolves_when_its_message_turns_up() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, sent) = account_with_sent(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("a reservation");
    drafts
        .set_state(draft.id, postio_model::DraftState::Unconfirmed)
        .expect("the state an interrupted submission leaves");

    // The copy the server filed, arriving in an ordinary sync.
    let mut message = postio_model::Message::new(account.id, sent, at(10));
    message.rfc_message_id = Some(reserved.clone());
    let message_id = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("the synced copy");

    let resolved = postio_sync::send::confirm_unconfirmed(&connection, account.id)
        .expect("confirming reads the store and nothing else");

    assert_eq!(
        resolved,
        vec![(draft.id, message_id)],
        "the draft and the message it turned out to be"
    );
    let after = drafts.get(draft.id).expect("read").expect("still there");
    assert_eq!(
        after.state,
        postio_model::DraftState::Sent,
        "the question is answered, so it stops being unconfirmed"
    );
    assert_eq!(
        drafts
            .by_message(message_id)
            .expect("read")
            .map(|found| found.id),
        Some(draft.id),
        "and the two are linked, so the surfaces that open one reach the other"
    );
}

/// The same lookup must not resolve on somebody else's message.
///
/// The reservation is the whole evidence here: matching on anything softer —
/// subject, recipients, a nearby timestamp — is how the wrong message gets
/// called the sent one, and the user is then told a message arrived that
/// never did.
#[tokio::test]
async fn an_unconfirmed_send_is_not_resolved_by_a_different_message() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, sent) = account_with_sent(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(&account, "grace@example.net");
    drafts.save(&mut draft).expect("save draft");
    drafts
        .queue_send(&mut draft, at(9))
        .expect("queue the send");
    drafts
        .set_state(draft.id, postio_model::DraftState::Unconfirmed)
        .expect("the state an interrupted submission leaves");

    // Same folder, same account, same everything but the id.
    let mut other = postio_model::Message::new(account.id, sent, at(10));
    other.rfc_message_id = Some(postio_model::RfcMessageId::new(
        "<someone-else@example.com>",
    ));
    MessageRepository::new(&connection)
        .create(&mut other)
        .expect("an unrelated message");

    let resolved =
        postio_sync::send::confirm_unconfirmed(&connection, account.id).expect("confirm");

    assert!(
        resolved.is_empty(),
        "only the reserved id is evidence: {resolved:?}"
    );
    assert_eq!(
        drafts
            .get(draft.id)
            .expect("read")
            .expect("still there")
            .state,
        postio_model::DraftState::Unconfirmed,
        "the question is still open, and saying otherwise would be a lie the \
         user acts on"
    );
}
