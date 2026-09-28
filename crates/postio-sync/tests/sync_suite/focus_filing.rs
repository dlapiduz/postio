//! Focus's filing pass (spec 007 T102, `contracts/engine.md`): the guards
//! read from the store, the writes made in the transaction that filed the
//! mail, and what it costs per message.
//!
//! The classifier here is a stand-in that files every stranger's mail as a
//! notification, asking every guard first -- the most a classifier can ask
//! of the store. What the built-in rules decide is T122's, and its suite is
//! `focus_rules.rs`; this one is about the pass that carries a decision out.

use std::sync::Arc;

use postio_account::backend::{AppendMessage, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_classify::{Facts, Layer, Outcome, Reason, ReasonKind};
use postio_model::{
    Account, EmailAddress, Identity, Mailbox, MailboxRole, MessageId, Operation, UidValidity,
};
use postio_storage::repository::{
    CorrespondentRepository, FilterDecisionRepository, FilterLayer, FilterReason,
    IdentityRepository, MessageRepository, OperationQueueRepository,
};
use postio_storage::test_support::{self, counting};
use postio_storage::{Checkout, Connection};
use postio_sync::{
    Classifier, FiledMessage, FilingEffects, FilingPass, FocusFiling, Outcome as Pass, SyncError,
    resync_mailbox_filing, sync_mailbox,
};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

/// Files every message no guard covers, as a notification from its
/// headers, asking each guard in turn -- the guards' own order, and all of
/// them for a stranger.
#[derive(Debug)]
struct FilesStrangers;

impl Classifier for FilesStrangers {
    fn at_filing(&self, filed: &FiledMessage<'_>, facts: &dyn Facts) -> Outcome {
        let senders = &filed.message.from;
        let guarded = senders.is_empty()
            || senders.iter().any(|sender| facts.never_filter(sender))
            || senders.iter().any(|sender| facts.own_domain(sender))
            || senders.iter().any(|sender| facts.wrote_to(sender))
            || filed.thread.is_none_or(|thread| facts.took_part(thread));
        if guarded || filed.role != MailboxRole::Inbox {
            return Outcome::default();
        }
        Outcome {
            filter: Some(Reason {
                kind: ReasonKind::Notification,
                source: None,
                layer: Layer::Header,
            }),
            ..Outcome::default()
        }
    }
}

fn pass() -> FocusFiling {
    FocusFiling::with_classifier(Arc::new(FilesStrangers))
}

/// A message from `from` to the account, `n` its subject and id.
fn mail(n: u32, from: &str, extra: &str) -> Vec<u8> {
    format!(
        "From: {from}\r\n\
         To: Test User <test@example.com>\r\n\
         Message-ID: <arrival-{n}@example.com>\r\n\
         {extra}Subject: Arrival {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

/// The store and a server with an inbox holding one old message, synced.
struct World {
    backend: MockBackend,
    _database: postio_storage::Store,
    connection: Checkout,
    account: Account,
    inbox: Mailbox,
    archive: Option<Mailbox>,
    sent: Mailbox,
}

async fn world(with_archive: bool) -> World {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(MockMessage::new(mail(0, "Old <old@example.org>", ""))),
        )
        .build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    let archive = if with_archive {
        Some(test_support::mailbox(&connection, &account, "Archive").await)
    } else {
        None
    };
    let sent = test_support::mailbox(&connection, &account, "Sent").await;
    assert_eq!(
        sent.role,
        MailboxRole::Sent,
        "the fixture needs a Sent folder"
    );
    if let Some(archive) = &archive {
        assert_eq!(archive.role, MailboxRole::Archive);
    }
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");
    World {
        backend,
        _database: database,
        connection,
        account,
        inbox,
        archive,
        sent,
    }
}

impl World {
    async fn deliver(&self, raw: Vec<u8>) {
        self.backend
            .append(INBOX, &AppendMessage::new(raw))
            .await
            .expect("deliver");
    }

    /// The next pass over the inbox, filed by `filing`: the arrivals.
    async fn pass(&self, filing: &dyn FilingPass) -> Vec<MessageId> {
        let outcome = resync_mailbox_filing(
            &self.connection,
            &self.backend,
            &self.inbox,
            Some(filing),
            &CancelToken::new(),
            |_| {},
        )
        .await
        .expect("an incremental pass");
        let Pass::Incremental { arrived, .. } = outcome else {
            panic!("expected an incremental pass, got {outcome:?}");
        };
        arrived
    }

    /// Where the message with this subject is now, and why it is there if
    /// Focus filed it away.
    async fn where_is(&self, subject: &str) -> (Mailbox, Option<(FilterReason, FilterLayer)>) {
        let id: i64 = postio_storage::sql::first(
            &self.connection,
            "SELECT id FROM messages WHERE subject = ?1",
            [subject],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a read")
        .unwrap_or_else(|| panic!("no message `{subject}`"));
        let message = MessageRepository::new(&self.connection)
            .get(MessageId::new(id))
            .await
            .expect("a read")
            .expect("the message");
        let mailbox = postio_storage::repository::MailboxRepository::new(&self.connection)
            .get(message.mailbox_id)
            .await
            .expect("a read")
            .expect("its mailbox");
        let decision = FilterDecisionRepository::new(&self.connection)
            .get(message.id)
            .await
            .expect("a read")
            .map(|decision| (decision.reason, decision.layer));
        (mailbox, decision)
    }

    /// A message the account sent to `to`, filed in Sent and counted as
    /// written, the way a local send files and counts it.
    async fn sent_to(&self, to: &str, message_id: &str) {
        let raw = format!(
            "From: Test User <test@example.com>\r\nTo: {to}\r\n\
             Message-ID: {message_id}\r\nSubject: From me\r\n\r\nHello.\r\n"
        );
        let mut copy = postio_model::mime::parse(raw.as_bytes()).into_message(
            self.account.id,
            self.sent.id,
            chrono::Utc::now(),
        );
        let id = MessageRepository::new(&self.connection)
            .create(&mut copy)
            .await
            .expect("the sent copy");
        postio_storage::repository::ThreadingRepository::new(&self.connection, self.account.id)
            .thread(&copy)
            .await
            .expect("threaded");
        CorrespondentRepository::new(&self.connection)
            .record_sent(self.account.id, &[id])
            .await
            .expect("counted");
    }
}

// --- What the pass does with a decision ------------------------------------------

#[tokio::test]
async fn a_filtered_arrival_is_archived_with_its_reason_and_the_move_queued() {
    // FR-110, FR-113, FR-117: archived as it is filed, with its reason and
    // the layer that decided, through the storage verbs in the transaction
    // that filed it, and the server told by a queued move.
    let world = world(true).await;
    world
        .deliver(mail(1, "Forge <notifications@forge.example>", ""))
        .await;

    let arrived = world.pass(&pass()).await;

    assert_eq!(arrived.len(), 1);
    let archive = world.archive.as_ref().expect("an archive");
    let (mailbox, decision) = world.where_is("Arrival 1").await;
    assert_eq!(mailbox.id, archive.id, "it left the inbox for the archive");
    assert_eq!(
        decision,
        Some((FilterReason::Notification, FilterLayer::Header)),
        "with its reason and its layer"
    );
    let queued = OperationQueueRepository::new(&world.connection)
        .pending(world.account.id, chrono::Utc::now())
        .await
        .expect("a read");
    assert_eq!(queued.len(), 1, "one operation for the server: {queued:?}");
    assert_eq!(
        queued[0].operation,
        Operation::Move {
            from: world.inbox.id,
            to: archive.id
        }
    );
}

#[tokio::test]
async fn an_account_with_no_archive_folder_keeps_its_mail_in_the_inbox() {
    // Filtered mail is archived (FR-117). With nowhere to archive it, it is
    // not filtered at all: a decision with the message still in the inbox
    // would say one thing and the list another.
    let world = world(false).await;
    world
        .deliver(mail(1, "Forge <notifications@forge.example>", ""))
        .await;

    world.pass(&pass()).await;

    let (mailbox, decision) = world.where_is("Arrival 1").await;
    assert_eq!(mailbox.id, world.inbox.id);
    assert_eq!(decision, None);
}

// --- The guards, as the store answers them (FR-111) -----------------------------------

#[tokio::test]
async fn each_guard_the_store_answers_keeps_mail_in_the_inbox() {
    let world = world(true).await;
    // The user wrote to Grace.
    world
        .sent_to("Grace <grace@example.net>", "<to-grace@example.com>")
        .await;
    // The user wrote in a conversation Oren now replies in; the user never
    // wrote to Oren himself.
    world
        .sent_to("Team <team@example.org>", "<mine@example.com>")
        .await;
    // A second identity at the user's own firm.
    let mut work = Identity::new(
        world.account.id,
        EmailAddress::new(Some("Test User"), "test@firm.example"),
    );
    IdentityRepository::new(&world.connection)
        .create(&mut work)
        .await
        .expect("an identity");

    world
        .deliver(mail(1, "Grace <grace@example.net>", ""))
        .await;
    world
        .deliver(mail(
            2,
            "Oren <oren@example.org>",
            "In-Reply-To: <mine@example.com>\r\nReferences: <mine@example.com>\r\n",
        ))
        .await;
    world
        .deliver(mail(3, "Colleague <colleague@firm.example>", ""))
        .await;
    world
        .deliver(mail(4, "Forge <notifications@forge.example>", ""))
        .await;

    let arrived = world.pass(&pass()).await;
    assert_eq!(arrived.len(), 4);

    for (subject, guard) in [
        ("Arrival 1", "the user wrote to the sender"),
        ("Arrival 2", "the user took part in the conversation"),
        ("Arrival 3", "the sender is at the user's own domain"),
    ] {
        let (mailbox, decision) = world.where_is(subject).await;
        assert_eq!(mailbox.id, world.inbox.id, "{guard}: it left the inbox");
        assert_eq!(decision, None, "{guard}: it has a decision");
    }
    let (mailbox, _) = world.where_is("Arrival 4").await;
    assert_eq!(
        Some(mailbox.id),
        world.archive.as_ref().map(|archive| archive.id),
        "and the stranger's notification, which no guard covers, is filed"
    );
}

// --- What it costs ------------------------------------------------------------------

/// Focus's pass, counted: what the statements it issued cost, per call.
#[derive(Debug)]
struct Counted {
    inner: FocusFiling,
    counts: std::sync::Mutex<Vec<(usize, counting::Counts)>>,
}

#[async_trait::async_trait]
impl FilingPass for Counted {
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        counting::reset();
        let effects = self.inner.file(transaction, filed).await;
        self.counts
            .lock()
            .expect("not poisoned")
            .push((filed.len(), counting::here()));
        effects
    }
}

#[tokio::test]
async fn the_pass_reads_at_most_four_statements_per_arrival_and_scans_no_mail() {
    // contracts/engine.md: at most 4 statements per new message, plus its
    // writes, with no scans. The stand-in asks every guard of every
    // stranger, which is the most a classifier can ask, and files each one,
    // which is the most a pass writes.
    let world = world(true).await;
    for n in 1..=6 {
        world
            .deliver(mail(n, &format!("Forge <notify-{n}@forge.example>"), ""))
            .await;
    }
    let counted = Counted {
        inner: pass(),
        counts: std::sync::Mutex::default(),
    };

    let arrived = world.pass(&counted).await;
    assert_eq!(arrived.len(), 6);
    let (_, decision) = world.where_is("Arrival 6").await;
    assert!(decision.is_some(), "the fixture files every arrival");

    let counts = counted.counts.lock().expect("not poisoned").clone();
    assert!(!counts.is_empty(), "the pass was called");
    println!("statements per call (arrivals, counts): {counts:?}");
    for (messages, cost) in &counts {
        assert!(
            cost.statements <= 4 * messages,
            "{} statements for {messages} arrivals",
            cost.statements
        );
    }

    // Every read it issues, asked of the planner: nothing walks the mail.
    // The user's own addresses are one read of the accounts and their
    // identities -- a handful of rows, once per call, never per message.
    let reads = FocusFiling::reads();
    assert!(!reads.is_empty());
    for sql in reads {
        // `plan` binds every placeholder, so a plan that did not come back
        // fails here rather than passing for having no steps.
        let plan = test_support::plan(&world.connection, &sql).await;
        assert!(!plan.is_empty(), "{sql}: no plan");
        let scanned: Vec<&str> = plan
            .lines()
            .filter(|step| step.trim_start().starts_with("SCAN"))
            .collect();
        assert!(
            scanned
                .iter()
                .all(|step| step.contains("accounts") || step.contains("identities")),
            "{sql}\n{plan}"
        );
    }
}
