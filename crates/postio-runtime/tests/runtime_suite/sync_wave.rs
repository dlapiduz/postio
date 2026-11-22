use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use postio_account::backend::{
    AppendMessage, FetchEvent, Held, MailBackend, MockBackend, MockMailbox, MockMessage,
};
use postio_core::bridge::event_channel;
use postio_model::operation::{Operation, OperationTarget};
use postio_model::{AccountId, MessageId};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::repository::{
    AccountRepository, ListQuery, ListScope, MailboxRepository, MessageRepository,
    OperationQueueRepository,
};
use postio_storage::test_support::TempStore;
use postio_storage::{BlobStore, Store, test_support};

use crate::harness;

use harness::BlobDir;

const LATENCY: Duration = Duration::from_millis(40);

fn message(n: u32) -> Vec<u8> {
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

fn folder(path: &str, attributes: &[&str], messages: u32) -> MockMailbox {
    let mut mailbox = MockMailbox::new(path);
    if !attributes.is_empty() {
        mailbox = mailbox.attributes(attributes.iter().copied());
    }
    for n in 1..=messages {
        mailbox = mailbox.message(MockMessage::new(message(n)));
    }
    mailbox
}

/// File-backed, not `test_support::memory()`: in-memory SQLite uses shared-cache locking, which
/// blocks a writer
/// behind a reader and serialises the lanes on the engine's one thread (#79).
async fn engine_over(backend: Arc<MockBackend>) -> (TempStore, Engine, BlobDir) {
    engine_over_with(backend, Default::default()).await
}

async fn engine_over_with(
    backend: Arc<MockBackend>,
    backfill: postio_runtime::BackfillPolicy,
) -> (TempStore, Engine, BlobDir) {
    let database = test_support::temp().await;
    let account = {
        let connection = database.connect().await.expect("a connection");
        test_support::account(&connection).await
    };
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    let engine = Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend,
        smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
            postio_account::secret::MemorySecretStore::default(),
        ))),
        events: sink,
        retry: Default::default(),
        backfill,
        reconnect: Default::default(),
        watch: Default::default(),
        network: NetworkSource::Ignored,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");
    let directory = BlobDir::new(engine.clone(), directory);
    (database, engine, directory)
}

async fn stored(database: &Store, path: &str) -> Option<u32> {
    let connection = database.connect().await.ok()?;
    let account = AccountRepository::new(&connection)
        .list()
        .await
        .ok()?
        .into_iter()
        .next()?;
    let mailbox = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .ok()?
        .into_iter()
        .find(|mailbox| mailbox.path == path)?;
    MessageRepository::new(&connection)
        .count(&ListQuery {
            scope: ListScope::Mailbox(mailbox.id),
            limit: 0,
            after: None,
        })
        .await
        .ok()
}

/// The deadline is a liveness bound only, deliberately enormous: a 30 s one became a performance
/// budget and flaked on loaded runners (#122, #125).
async fn until<F, Fut>(what: &str, condition: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let waited = tokio::time::timeout(Duration::from_secs(180), async {
        while !condition().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "timed out waiting for {what}");
}

async fn bodies_missing(database: &Store, path: &str) -> Option<usize> {
    let connection = database.connect().await.ok()?;
    let account = AccountRepository::new(&connection)
        .list()
        .await
        .ok()?
        .into_iter()
        .next()?;
    let mailbox = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .ok()?
        .into_iter()
        .find(|mailbox| mailbox.path == path)?;
    MessageRepository::new(&connection)
        .needing_backfill_from(mailbox.id, u32::MAX, 0)
        .await
        .ok()
        .map(|candidates| candidates.len())
}

fn archive_fetches(backend: &MockBackend) -> usize {
    backend
        .header_fetches()
        .iter()
        .filter(|mailbox| *mailbox == "Archive")
        .count()
}

/// An order on the server's call log, not a stopwatch: INBOX's first header fetch before the
/// archive's last.
/// First against last because the watcher may fetch INBOX again later (#125).
fn inbox_was_not_queued_behind_the_archive(log: &[String]) -> bool {
    let first_inbox = log.iter().position(|mailbox| mailbox == "INBOX");
    let last_archive = log.iter().rposition(|mailbox| mailbox == "Archive");
    match (first_inbox, last_archive) {
        (Some(inbox), Some(archive)) => inbox < archive,
        _ => false,
    }
}

#[test]
fn the_order_check_rejects_a_sequential_archive_first_engine() {
    let starved: Vec<String> = std::iter::repeat_n("Archive".to_owned(), 50)
        .chain(std::iter::once("INBOX".to_owned()))
        .collect();
    assert!(!inbox_was_not_queued_behind_the_archive(&starved));

    let healthy: Vec<String> = ["Archive", "INBOX", "Archive", "Archive"]
        .map(str::to_owned)
        .to_vec();
    assert!(inbox_was_not_queued_behind_the_archive(&healthy));

    assert!(!inbox_was_not_queued_behind_the_archive(&[
        "Archive".to_owned()
    ]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_sync_asks_about_several_mailboxes_at_once() {
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 40))
            .mailbox(folder("Sent Messages", &["\\Sent"], 40))
            .mailbox(folder("Archive", &["\\Archive"], 40))
            .mailbox(folder("Deleted Messages", &["\\Trash"], 40))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine, _directory) = engine_over(backend.clone()).await;

    until("every folder to finish its first sync", async || {
        for path in ["INBOX", "Sent Messages", "Archive", "Deleted Messages"] {
            if stored(&database, path).await != Some(40) {
                return false;
            }
        }
        true
    })
    .await;

    assert!(
        backend.peak_in_flight() > 1,
        "the engine never had more than one request in front of the server, so \
         it is still syncing one mailbox at a time: peak {}",
        backend.peak_in_flight()
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_inbox_finishes_while_a_large_archive_is_still_going() {
    // Concurrency must not cost INBOX-first: the wave must keep taking mailboxes off the front of a
    // priority-ordered queue. No background lane: its bodies were most of the case's time.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.set_latency(LATENCY);

    let (database, engine, _directory) = engine_over_with(
        backend.clone(),
        postio_runtime::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;

    // Liveness only: wait for both, then judge the order the server saw, which keeps the assertion
    // non-vacuous.
    until("both folders to finish their first sync", async || {
        stored(&database, "INBOX").await == Some(10)
            && stored(&database, "Archive").await == Some(400)
    })
    .await;

    let log = backend.header_fetches();
    assert!(
        inbox_was_not_queued_behind_the_archive(&log),
        "INBOX's first header fetch came after the archive's last, so INBOX \
         waited out the whole archive: {log:?}"
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_is_served_without_waiting_out_the_wave_it_arrived_during() {
    // A wave is background work and the user's request must not queue behind it (#122). Causal:
    // archive fetches must still be served
    // after the answer, which fails if the wave ran to completion first. The archive's pass is
    // parked inside its second header fetch.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.set_latency(LATENCY);
    backend.hold(Held::Headers, "Archive", 1);

    let (database, engine, _directory) = engine_over_with(
        backend.clone(),
        postio_runtime::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;

    until("the archive's pass to be parked mid-flight", async || {
        backend.parked() == 1
    })
    .await;

    tokio::time::timeout(Duration::from_secs(180), engine.backfill_progress())
        .await
        .expect("the engine answers while the archive's pass is parked")
        .expect("the engine answers while it is syncing");
    let fetches_at_answer = archive_fetches(&backend);

    backend.release();
    until("the archive to finish anyway", async || {
        stored(&database, "Archive").await == Some(400)
    })
    .await;

    let fetches_in_the_end = archive_fetches(&backend);
    assert!(
        fetches_at_answer < fetches_in_the_end,
        "no archive fetch was served after the answer came back, so the \
         question waited out the wave rather than interrupting it \
         ({fetches_at_answer} of {fetches_in_the_end} archive fetches had \
         already happened)"
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_bodies_start_before_the_archive_s_headers_finish() {
    // #631: INBOX's bodies must not wait for the archive's headers. The archive's pass is parked
    // inside its
    // second header fetch so its headers cannot finish first.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.set_latency(LATENCY);
    backend.hold(Held::Headers, "Archive", 1);

    let (_database, engine, _directory) = engine_over(backend.clone()).await;

    until(
        "INBOX's first body while the archive's headers are parked",
        async || {
            backend.parked() == 1
                && backend
                    .body_fetches()
                    .iter()
                    .any(|mailbox| mailbox == "INBOX")
        },
    )
    .await;
    assert_eq!(
        archive_fetches(&backend),
        1,
        "the archive's headers were let go before INBOX's first body"
    );
    backend.release();
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_bodies_keep_arriving_while_the_archive_s_headers_run() {
    // #631 again: the rest of INBOX can only arrive through a top-up during the wave, which lasts
    // as long as its largest folder.
    // Causal: the archive must still be fetching headers after the last INBOX body lands; its pass
    // is parked (#2166).
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 28))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    backend.hold(Held::Headers, "Archive", 1);

    let policy = postio_runtime::BackfillPolicy {
        seed_batch: 20,
        ..Default::default()
    };
    let (database, engine, _directory) = engine_over_with(backend.clone(), policy).await;

    until(
        "INBOX's headers to land while the archive is under way",
        async || stored(&database, "INBOX").await == Some(28) && backend.parked() == 1,
    )
    .await;

    until("every INBOX body to be local", async || {
        bodies_missing(&database, "INBOX").await == Some(0)
    })
    .await;
    assert_eq!(
        archive_fetches(&backend),
        1,
        "the archive's headers were let go before INBOX's last body arrived"
    );
    backend.release();
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_inbox_mail_is_synced_while_the_archive_s_headers_run() {
    // Nothing looked for new mail while a wave ran, because `keep_watch` sits below the wave loop.
    // Causal: the
    // archive's fetch count when the message lands must be below its final count; its pass is
    // parked until then (#2166).
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    backend.hold(Held::Headers, "Archive", 1);

    let (database, engine, _directory) = engine_over(backend.clone()).await;

    until(
        "INBOX's headers to land while the archive is under way",
        async || stored(&database, "INBOX").await == Some(10) && backend.parked() == 1,
    )
    .await;

    backend
        .append("INBOX", &AppendMessage::new(message(11)))
        .await
        .expect("the mock accepts new mail");

    until("the new message to be synced", async || {
        stored(&database, "INBOX").await == Some(11)
    })
    .await;
    assert_eq!(
        archive_fetches(&backend),
        1,
        "the archive's headers were let go before the new message was synced"
    );
    backend.release();
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn new_inbox_mail_is_synced_while_the_wave_only_fetches_bodies() {
    // The same promise after the headers are done: only a committed header batch looked at INBOX.
    // Causal: the archive must still be missing bodies when the message lands.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 600))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    // The archive's bodies are parked until the case is over, so the body-only stretch outlasts the
    // question on any load (#2166).
    backend.hold(Held::Bodies, "Archive", 0);

    let (database, engine, _directory) = engine_over(backend.clone()).await;

    until(
        "every header to land, with the archive's bodies on the wire",
        async || {
            stored(&database, "INBOX").await == Some(10)
                && stored(&database, "Archive").await == Some(600)
                && backend.parked() > 0
        },
    )
    .await;

    backend
        .append("INBOX", &AppendMessage::new(message(11)))
        .await
        .expect("the mock accepts new mail");

    until("the new message to be synced", async || {
        stored(&database, "INBOX").await == Some(11)
    })
    .await;
    assert_eq!(
        bodies_missing(&database, "Archive").await,
        Some(600),
        "an archive body came down while every one was held"
    );
    backend.release();
    drop(engine);
}

fn a_note_with_a_large_attachment() -> MockMessage {
    // "AAA" is "QUFB" in base64, so the payload can be written without an encoder.
    const LINES: usize = 105_264;
    let decoded = (LINES * 19 * 3) as u64;
    let line = format!("{}\r\n", "QUFB".repeat(19));
    let encoded = line.repeat(LINES);
    let raw = format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         To: Postio <postio@example.net>\r\n\
         Subject: The plans\r\n\
         Message-ID: <plans@example.com>\r\n\
         Date: Wed, 1 Jul 2026 09:00:00 +0000\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\
         \r\n\
         --b\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         The plans are attached.\r\n\
         --b\r\n\
         Content-Type: application/pdf; name=plans.pdf\r\n\
         Content-Transfer-Encoding: base64\r\n\
         Content-Disposition: attachment; filename=plans.pdf\r\n\
         \r\n\
         {encoded}\
         --b--\r\n"
    );
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "text/plain", 23)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "application/pdf", decoded)
                .with_encoding("base64")
                .with_filename("plans.pdf"),
        ],
    );
    MockMessage::new(raw.into_bytes())
        .with_structure(structure)
        .with_part("1", &b"The plans are attached."[..])
        .with_part("2", encoded.into_bytes())
}

async fn readable_with_attachments(database: &Store, message: MessageId) -> bool {
    let Ok(connection) = database.connect().await else {
        return false;
    };
    let Ok(Some(message)) = MessageRepository::new(&connection).get(message).await else {
        return false;
    };
    !matches!(
        message.sync.body_state,
        postio_model::BodyState::NotFetched | postio_model::BodyState::HeadersOnly
    ) && !message.attachments.is_empty()
        && message
            .attachments
            .iter()
            .all(|attachment| attachment.blob_id.is_some())
}

#[tokio::test(flavor = "multi_thread")]
async fn new_mail_and_its_attachments_come_down_ahead_of_the_backlog() {
    // New mail used to join the back of the queue, and a note over the five-megabyte cap never had
    // its words fetched;
    // its text and attachment must be local while the archive still misses bodies.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 600))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    backend.hold(Held::Bodies, "Archive", 0);

    let (database, engine, _directory) = engine_over(backend.clone()).await;

    until("every header to land", async || {
        stored(&database, "INBOX").await == Some(10)
            && stored(&database, "Archive").await == Some(600)
    })
    .await;
    assert_eq!(
        bodies_missing(&database, "Archive").await,
        Some(600),
        "an archive body came down while every one was held"
    );
    // Let go slowly: the note has to overtake a backlog that is moving.
    backend.set_latency(Duration::from_millis(150));
    backend.release();

    backend.deliver("INBOX", a_note_with_a_large_attachment());
    until("the new message to be synced", async || {
        stored(&database, "INBOX").await == Some(11)
    })
    .await;
    let (_, arrived) = newest_message(&database, "INBOX").await;

    until(
        "the new message's text and attachment to be local",
        async || readable_with_attachments(&database, arrived).await,
    )
    .await;
    let missing = bodies_missing(&database, "Archive").await.unwrap_or(0);
    assert!(
        missing > 0,
        "the new message became readable only once the archive's bodies were \
         all down, so it waited behind the whole backlog"
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_pass_does_not_hold_the_folders_queued_behind_the_wave() {
    // A slow high-priority folder shares the first wave and regular folders queue behind it; a wave
    // that returned only when every
    // pass finished left a lane free all that time. Creates are refused so reserved-role folders do
    // not outrank the regular ones.
    // Causal: when the last regular folder finishes its first sync the slow pass must still be
    // running.
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Drafts", &["\\Drafts"], 400))
            .mailbox(folder("Lists/alpha", &[], 5))
            .mailbox(folder("Lists/bravo", &[], 5))
            .mailbox(folder("Lists/carol", &[], 5))
            .mailbox(folder("Lists/dana", &[], 5))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    // The slow pass is parked inside its second header fetch until every regular folder is in, so
    // it is slow on any machine (#2166).
    backend.hold(Held::Headers, "Drafts", 1);
    let (database, engine, _directory) = engine_over_with(
        backend.clone(),
        postio_runtime::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;

    until(
        "the regular folders to finish while the slow pass is parked",
        async || {
            for path in ["Lists/alpha", "Lists/bravo", "Lists/carol", "Lists/dana"] {
                if stored(&database, path).await != Some(5) {
                    return false;
                }
            }
            backend.parked() == 1
        },
    )
    .await;
    backend.release();

    until("every folder to finish its first sync", async || {
        for (path, count) in [
            ("Drafts", 400),
            ("Lists/alpha", 5),
            ("Lists/bravo", 5),
            ("Lists/carol", 5),
            ("Lists/dana", 5),
        ] {
            if stored(&database, path).await != Some(count) {
                return false;
            }
        }
        true
    })
    .await;

    let order = backend.fetch_order();
    let first_regular = order
        .iter()
        .position(
            |event| matches!(event, FetchEvent::Header(mailbox) if mailbox.starts_with("Lists/")),
        )
        .expect("the regular folders were fetched");
    let last_drafts = order
        .iter()
        .rposition(|event| matches!(event, FetchEvent::Header(mailbox) if mailbox == "Drafts"))
        .expect("the slow folder was fetched");
    assert!(
        first_regular < last_drafts,
        "no regular folder was fetched until the slow pass had finished \
         (first regular header at {first_regular}, last Drafts header at \
         {last_drafts} of {} events), so the free lane was never refilled \
         and the queue waited out the slowest pass",
        order.len()
    );
    drop(engine);
}

fn archive_listings(backend: &MockBackend) -> usize {
    backend
        .uid_listings()
        .iter()
        .filter(|mailbox| *mailbox == "Archive")
        .count()
}

async fn newest_message(database: &Store, path: &str) -> (AccountId, MessageId) {
    let connection = database.connect().await.expect("a connection");
    let account = AccountRepository::new(&connection)
        .list()
        .await
        .expect("the accounts")
        .into_iter()
        .next()
        .expect("an account");
    let mailbox = MailboxRepository::new(&connection)
        .list_for_account(account.id)
        .await
        .expect("the folders")
        .into_iter()
        .find(|mailbox| mailbox.path == path)
        .expect("the folder exists locally");
    let page = MessageRepository::new(&connection)
        .page(&ListQuery {
            scope: ListScope::Mailbox(mailbox.id),
            limit: 1,
            after: None,
        })
        .await
        .expect("a page");
    (account.id, page.first().expect("the folder has mail").id)
}

async fn body_is_local(database: &Store, message: MessageId) -> bool {
    let Ok(connection) = database.connect().await else {
        return false;
    };
    MessageRepository::new(&connection)
        .backfill_candidate(message)
        .await
        .is_ok_and(|candidate| candidate.is_none())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_body_requested_during_the_wave_does_not_restart_its_lanes() {
    // #1593: a body request cancelled every lane and restarted first-sync passes once per message
    // opened. Causal on the
    // mock's call log: a pass lists its UIDs once at start, so a second listing is the restart; the
    // body must land while the archive is still fetching (#944).
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    backend.hold(Held::Headers, "Archive", 1);
    // No background lane: the request must be the only reason a body is fetched.
    let (database, engine, _directory) = engine_over_with(
        backend.clone(),
        postio_runtime::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;

    until("the archive's pass to be parked mid-flight", async || {
        stored(&database, "INBOX").await == Some(10) && backend.parked() == 1
    })
    .await;
    let (_, message) = newest_message(&database, "INBOX").await;
    assert!(
        !body_is_local(&database, message).await,
        "the body was already local, so nothing below would ask for anything"
    );

    let asked = engine
        .request_body(message)
        .await
        .expect("the engine answers while it is syncing");
    assert!(asked, "the engine found nothing to fetch for that message");

    until("the requested body to land", async || {
        body_is_local(&database, message).await
    })
    .await;
    assert_eq!(
        backend.parked(),
        1,
        "the archive's parked fetch ended while the body was fetched: the \
         wave cancelled its lanes to serve a body request"
    );
    let listings_when_it_landed = archive_listings(&backend);

    backend.release();
    until("the archive to finish anyway", async || {
        stored(&database, "Archive").await == Some(400)
    })
    .await;
    assert_eq!(
        listings_when_it_landed, 1,
        "the archive had been listed {listings_when_it_landed} times when the \
         body landed: its pass was cancelled and started again to serve a \
         body request"
    );
    assert_eq!(
        archive_listings(&backend),
        1,
        "the archive's pass was restarted after the body landed"
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_write_during_the_wave_is_drained_without_restarting_its_lanes() {
    // #1593: a mark-read is a queue row, and the wave answered it by cancelling every lane; a drain
    // can run beside them.
    // The archive's pass is let through one batch and parked, so the wave is still running when the
    // queue poll notices (#2105).
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(folder("INBOX", &[], 10))
            .mailbox(folder("Archive", &["\\Archive"], 400))
            .build(),
    );
    backend.refuse_creates("no new folders here");
    backend.set_latency(LATENCY);
    backend.hold(Held::Headers, "Archive", 1);

    // No background lane: its bodies are not what this asks about.
    let (database, engine, _directory) = engine_over_with(
        backend.clone(),
        postio_runtime::BackfillPolicy {
            background: false,
            ..Default::default()
        },
    )
    .await;

    until("the archive's pass to be parked mid-flight", async || {
        stored(&database, "INBOX").await == Some(10) && backend.parked() == 1
    })
    .await;
    let (account, message) = newest_message(&database, "INBOX").await;
    {
        let connection = database.connect().await.expect("a connection");
        OperationQueueRepository::new(&connection)
            .enqueue(
                account,
                OperationTarget::Message(message),
                &Operation::SetFlags {
                    flags: postio_model::FlagSet::from_iter([postio_model::Flag::Seen]),
                },
                Utc::now(),
            )
            .await
            .expect("the write is queued");
    }

    until("the queued write to be drained", async || {
        let Ok(connection) = database.connect().await else {
            return false;
        };
        OperationQueueRepository::new(&connection)
            .pending(account, Utc::now())
            .await
            .is_ok_and(|due| due.is_empty())
    })
    .await;
    assert_eq!(
        backend.parked(),
        1,
        "the archive's parked fetch ended while the write was drained: the \
         wave cancelled its lanes to drain one queued operation"
    );
    let listings_when_drained = archive_listings(&backend);
    assert_eq!(
        listings_when_drained, 1,
        "the archive had been listed {listings_when_drained} times when the \
         write was drained: its pass was cancelled and started again to \
         drain one queued operation"
    );

    backend.release();
    until("the archive to finish anyway", async || {
        stored(&database, "Archive").await == Some(400)
    })
    .await;
    assert_eq!(
        archive_listings(&backend),
        1,
        "the archive's pass was restarted after the drain"
    );
    drop(engine);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_inbox_syncs_its_headers_and_newest_bodies_before_any_other_mailbox() {
    // #1709: Focus's whole screen is the inbox. As an order on the server's call log: every INBOX
    // header fetch precedes every other
    // fetch, and INBOX's seed-batch bodies precede every other mailbox's header and body fetch.
    const INBOX: u32 = 300;
    const OTHERS: [(&str, &[&str], u32); 5] = [
        ("Archive", &["\\Archive"], 1_500),
        ("Sent Messages", &["\\Sent"], 800),
        ("Projects", &[], 600),
        ("Receipts", &[], 600),
        ("Travel", &[], 600),
    ];
    let mut builder = MockBackend::builder().mailbox(folder("INBOX", &[], INBOX));
    for (path, attributes, count) in OTHERS {
        builder = builder.mailbox(folder(path, attributes, count));
    }
    let backend = Arc::new(builder.build());
    backend.refuse_creates("no new folders here");
    backend.set_latency(Duration::from_millis(5));

    let policy = postio_runtime::BackfillPolicy::default();
    let newest_page = policy.seed_batch as usize;
    assert!(
        newest_page < INBOX as usize,
        "the page must be a part of INBOX"
    );
    let (database, engine, _directory) = engine_over_with(backend.clone(), policy).await;

    until("INBOX to hold every header", async || {
        stored(&database, "INBOX").await == Some(INBOX)
    })
    .await;
    let mut unfinished = Vec::new();
    for (path, _, count) in OTHERS {
        unfinished.push((path, stored(&database, path).await.unwrap_or(0) < count));
    }
    {
        let connection = database.connect().await.expect("a connection");
        let account = AccountRepository::new(&connection)
            .list()
            .await
            .expect("accounts")
            .remove(0);
        let inbox = MailboxRepository::new(&connection)
            .list_for_account(account.id)
            .await
            .expect("folders")
            .into_iter()
            .find(|mailbox| mailbox.path == "INBOX")
            .expect("INBOX is discovered");
        let page = MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox.id),
                limit: INBOX,
                after: None,
            })
            .await
            .expect("the list query");
        assert_eq!(
            page.len(),
            INBOX as usize,
            "the inbox list is not complete while other mailboxes sync"
        );
    }
    assert!(
        unfinished.iter().all(|(_, unfinished)| *unfinished),
        "another mailbox had already finished when INBOX's last header landed: {unfinished:?}"
    );

    until(
        "the other mailboxes to begin and INBOX's bodies to land",
        async || {
            let order = backend.fetch_order();
            let inbox_bodies = order
                .iter()
                .filter(|event| matches!(event, FetchEvent::Body(path) if path == "INBOX"))
                .count();
            let others_begun = order
                .iter()
                .any(|event| matches!(event, FetchEvent::Header(path) if path != "INBOX"));
            others_begun && inbox_bodies >= newest_page
        },
    )
    .await;

    let order = backend.fetch_order();
    let is_inbox = |event: &FetchEvent| matches!(event, FetchEvent::Header(path) | FetchEvent::Body(path) if path == "INBOX");
    let last_inbox_header = order
        .iter()
        .rposition(|event| matches!(event, FetchEvent::Header(path) if path == "INBOX"))
        .expect("INBOX headers were fetched");
    let first_other = order
        .iter()
        .position(|event| !is_inbox(event))
        .expect("other mailboxes were fetched");
    assert!(
        last_inbox_header < first_other,
        "another mailbox's fetch began before INBOX's headers were done: \
         INBOX's last header was call {last_inbox_header}, the first other \
         fetch was {first_other}"
    );
    let inbox_bodies: Vec<usize> = order
        .iter()
        .enumerate()
        .filter(|(_, event)| matches!(event, FetchEvent::Body(path) if path == "INBOX"))
        .map(|(position, _)| position)
        .collect();
    assert!(
        inbox_bodies.len() >= newest_page,
        "INBOX's newest page of bodies was never fetched: {} of {newest_page}",
        inbox_bodies.len()
    );
    assert!(
        inbox_bodies[newest_page - 1] < first_other,
        "another mailbox's fetch began before INBOX's newest {newest_page} \
         bodies were down: the {newest_page}th INBOX body was call {}, the \
         first other fetch was {first_other}",
        inbox_bodies[newest_page - 1]
    );
    drop(engine);
}
