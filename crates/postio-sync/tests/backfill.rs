//! Backfilling message bodies: newest first, out of the user's way, and never
//! in front of the message they just opened.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_imap::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_imap::cancel::CancelToken;
use postio_model::{BodyState, Mailbox, MessageId, Uid, UidValidity};
use postio_storage::BlobStore;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support::{self, TempDatabase};
use postio_sync::backfill::{Backfill, BackfillPolicy, BodyRequest, Outcome, fetch_body};
use postio_sync::sync_mailbox;

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second)
}

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\
         Message-ID: <note-{n}@example.com>\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         The body of note {n}.\r\n"
    )
    .into_bytes()
}

async fn server(count: u32) -> MockBackend {
    let mut inbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        inbox = inbox.message(MockMessage::new(note(n)).with_internal_date(at(n as i64)));
    }
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");
    backend
}

/// A file-backed database and a blob store beside it, because a blob store is
/// a directory and an in-memory database has no directory to sit next to.
struct Local {
    database: TempDatabase,
    connection: postio_storage::PooledConnection,
    blobs: BlobStore,
    inbox: Mailbox,
}

fn local() -> Local {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);
    let blobs = BlobStore::open(database.directory().join("blobs")).expect("a blob store");
    Local {
        database,
        connection,
        blobs,
        inbox,
    }
}

fn policy() -> BackfillPolicy {
    BackfillPolicy::default()
}

/// A request for `uid`, received `uid` seconds into the fixture's timeline, so
/// a higher UID is also the newer message.
fn request(mailbox: &Mailbox, id: MessageId, uid: u32, size: u64) -> BodyRequest {
    BodyRequest {
        message: id,
        mailbox: mailbox.id,
        path: mailbox.path.clone(),
        uid: Uid::new(uid),
        size,
        received_at: at(uid as i64),
        whole: false,
    }
}

/// Syncs headers so the local rows exist, and returns them oldest UID first.
async fn headers(local: &Local, backend: &MockBackend) -> Vec<(MessageId, u32)> {
    sync_mailbox(
        &local.connection,
        backend,
        &local.inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("headers");

    let messages = MessageRepository::new(&local.connection);
    let mut rows: Vec<(MessageId, u32)> = messages
        .uids_in(local.inbox.id, UidValidity::new(VALIDITY))
        .expect("uids")
        .into_iter()
        .map(|uid| {
            let message = messages
                .by_uid(local.inbox.id, UidValidity::new(VALIDITY), uid)
                .expect("look up")
                .expect("stored");
            (message.id, uid.get())
        })
        .collect();
    rows.sort_by_key(|(_, uid)| *uid);
    rows
}

// ---------------------------------------------------------------------------
// The user always wins — the acceptance criterion
// ---------------------------------------------------------------------------

#[test]
fn opening_a_message_jumps_the_whole_backlog() {
    let local = local();
    let mut backfill = Backfill::new(policy());

    for uid in 1..=200 {
        backfill.enqueue(request(
            &local.inbox,
            MessageId::new(uid as i64),
            uid,
            1_024,
        ));
    }
    let opened = MessageId::new(7);

    backfill.request_now(request(&local.inbox, opened, 7, 1_024));

    assert_eq!(
        backfill.next_body().expect("work").request.message,
        opened,
        "a backlog of two hundred messages must not stand between the user and \
         the one they just clicked"
    );
}

#[test]
fn the_backlog_never_starves_the_user_more_than_one_body_at_a_time() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    for uid in 1..=50 {
        backfill.enqueue(request(
            &local.inbox,
            MessageId::new(uid as i64),
            uid,
            1_024,
        ));
    }

    // One background body is already in flight; the user opens something.
    let in_flight = backfill.next_body().expect("background work");
    let opened = MessageId::new(500);
    backfill.request_now(request(&local.inbox, opened, 500, 1_024));
    backfill.finished(in_flight.request.message, Outcome::Stored { bytes: 1_024 });

    assert_eq!(
        backfill.next_body().expect("work").request.message,
        opened,
        "the longest the user can wait is the one body already on the wire"
    );
}

#[test]
fn a_queued_message_the_user_opens_is_promoted_rather_than_fetched_twice() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    // The *oldest* of the three, so it would be last out of the backlog and
    // the assertion cannot pass by accident.
    let message = MessageId::new(1);

    backfill.enqueue(request(&local.inbox, message, 1, 1_024));
    backfill.enqueue(request(&local.inbox, MessageId::new(3), 3, 1_024));
    backfill.enqueue(request(&local.inbox, MessageId::new(2), 2, 1_024));
    assert_eq!(backfill.progress().pending, 3);

    backfill.request_now(request(&local.inbox, message, 1, 1_024));

    assert_eq!(backfill.progress().pending, 3, "promoted, not duplicated");
    assert_eq!(backfill.next_body().expect("work").request.message, message);
    assert_eq!(backfill.progress().pending, 2);
}

#[test]
fn a_message_already_in_flight_is_not_handed_out_again() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, 1_024));

    let claim = backfill.next_body().expect("work");
    backfill.request_now(request(&local.inbox, claim.request.message, 1, 1_024));

    assert!(
        backfill.next_body().is_none(),
        "asking for a body already on the wire must not put a second copy of \
         it on the wire"
    );
}

// ---------------------------------------------------------------------------
// Newest first
// ---------------------------------------------------------------------------

#[test]
fn the_backlog_is_worked_newest_first() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    for uid in [3, 1, 5, 2, 4] {
        backfill.enqueue(request(
            &local.inbox,
            MessageId::new(uid as i64),
            uid,
            1_024,
        ));
    }

    let mut order = Vec::new();
    while let Some(claim) = backfill.next_body() {
        order.push(claim.request.uid.get());
        backfill.finished(claim.request.message, Outcome::Stored { bytes: 1_024 });
    }

    assert_eq!(
        order,
        vec![5, 4, 3, 2, 1],
        "nobody opens a mail client to read the oldest message in it"
    );
}

// ---------------------------------------------------------------------------
// The size cap
// ---------------------------------------------------------------------------

#[test]
fn a_body_over_the_cap_is_left_on_the_server_until_it_is_wanted() {
    let local = local();
    let cap = 1_000_000;
    let mut backfill = Backfill::new(BackfillPolicy {
        max_body_bytes: Some(cap),
        ..policy()
    });
    let huge = MessageId::new(1);

    backfill.enqueue(request(&local.inbox, huge, 1, cap + 1));

    assert!(
        backfill.next_body().is_none(),
        "too big to fetch speculatively"
    );
    assert_eq!(backfill.progress().skipped, 1);

    // …until the user opens it, at which point the cap is beside the point:
    // they are looking at a spinner and they asked for this.
    backfill.request_now(request(&local.inbox, huge, 1, cap + 1));
    assert_eq!(backfill.next_body().expect("work").request.message, huge);
}

#[test]
fn no_cap_means_no_cap() {
    let local = local();
    let mut backfill = Backfill::new(BackfillPolicy {
        max_body_bytes: None,
        ..policy()
    });

    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, u64::MAX));

    assert!(backfill.next_body().is_some());
}

// ---------------------------------------------------------------------------
// Getting out of the way
// ---------------------------------------------------------------------------

#[test]
fn a_metered_connection_pauses_the_backlog_but_never_the_user() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, 1_024));

    backfill.set_metered(true);
    assert!(
        backfill.next_body().is_none(),
        "speculative megabytes over a phone's tether are the user's money"
    );

    let opened = MessageId::new(2);
    backfill.request_now(request(&local.inbox, opened, 2, 1_024));
    assert_eq!(
        backfill.next_body().expect("work").request.message,
        opened,
        "the message the user is looking at is not speculative"
    );

    backfill.set_metered(false);
    assert!(backfill.next_body().is_some(), "and the backlog resumes");
}

#[test]
fn an_active_user_pauses_the_backlog() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, 1_024));

    backfill.set_user_active(true);
    assert!(backfill.next_body().is_none());

    backfill.set_user_active(false);
    assert!(backfill.next_body().is_some());
}

#[test]
fn a_policy_with_the_background_lane_off_still_serves_the_user() {
    let local = local();
    let mut backfill = Backfill::new(BackfillPolicy {
        background: false,
        ..policy()
    });

    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, 1_024));
    assert!(backfill.next_body().is_none());

    backfill.request_now(request(&local.inbox, MessageId::new(2), 2, 1_024));
    assert!(
        backfill.next_body().is_some(),
        "`[sync] body_fetch` decides whether bodies are pulled ahead of time, \
         not whether the reading pane works"
    );
}

// ---------------------------------------------------------------------------
// Cancellation and progress — the acceptance criterion
// ---------------------------------------------------------------------------

#[test]
fn cancelling_stops_the_backlog_and_whatever_is_on_the_wire() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    for uid in 1..=10 {
        backfill.enqueue(request(
            &local.inbox,
            MessageId::new(uid as i64),
            uid,
            1_024,
        ));
    }
    let claim = backfill.next_body().expect("work");

    backfill.cancel();

    assert!(
        claim.cancel.is_cancelled(),
        "the fetch in flight is stopped"
    );
    assert!(backfill.next_body().is_none());
    assert_eq!(backfill.progress().pending, 0);
    assert!(backfill.is_cancelled());
}

#[test]
fn a_cancelled_backfill_can_be_restarted() {
    let local = local();
    let mut backfill = Backfill::new(policy());
    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, 1_024));
    backfill.cancel();

    backfill.restart();
    backfill.enqueue(request(&local.inbox, MessageId::new(1), 1, 1_024));

    let claim = backfill.next_body().expect("work");
    assert!(!claim.cancel.is_cancelled());
}

#[test]
fn progress_accounts_for_every_message_that_went_in() {
    let local = local();
    let mut backfill = Backfill::new(BackfillPolicy {
        max_body_bytes: Some(2_048),
        ..policy()
    });

    for uid in 1..=4 {
        backfill.enqueue(request(
            &local.inbox,
            MessageId::new(uid as i64),
            uid,
            1_024,
        ));
    }
    backfill.enqueue(request(&local.inbox, MessageId::new(9), 9, 4_096));

    let first = backfill.next_body().expect("work");
    backfill.finished(first.request.message, Outcome::Stored { bytes: 1_024 });
    let second = backfill.next_body().expect("work");
    backfill.finished(second.request.message, Outcome::Gone);
    let third = backfill.next_body().expect("work");
    backfill.finished(
        third.request.message,
        Outcome::Failed {
            reason: "the server hung up".to_owned(),
        },
    );

    let progress = backfill.progress();
    assert_eq!(progress.stored, 1);
    assert_eq!(progress.gone, 1);
    assert_eq!(progress.failed, 1);
    assert_eq!(progress.skipped, 1, "the one over the cap");
    assert_eq!(progress.bytes, 1_024);
    assert_eq!(progress.pending, 1);
    assert_eq!(progress.in_flight, 0);
    assert!(!backfill.is_idle());

    let last = backfill.next_body().expect("work");
    backfill.finished(last.request.message, Outcome::Stored { bytes: 1_024 });
    assert!(backfill.is_idle());
}

// ---------------------------------------------------------------------------
// Actually fetching one
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fetching_a_body_stores_the_raw_message_and_its_decoded_text() {
    let backend = server(2).await;
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[1];

    let messages = MessageRepository::new(&local.connection);
    assert_eq!(
        messages.get(id).expect("get").expect("row").sync.body_state,
        BodyState::HeadersOnly,
        "the header pass leaves the body on the server"
    );

    let outcome = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(matches!(outcome, Outcome::Stored { .. }));

    let stored = messages.get(id).expect("get").expect("row");
    assert_eq!(stored.sync.body_state, BodyState::Full);
    let raw = stored.raw_blob_id.expect("the raw message is kept");
    assert_eq!(
        local.blobs.get(&raw).expect("read the blob"),
        note(uid),
        "the bytes the server sent, verbatim, so a future parser change can \
         re-read them"
    );

    let blobs = messages.body_blobs(id).expect("blobs").expect("the row");
    let text = blobs.text.expect("a text/plain body");
    assert_eq!(
        String::from_utf8(local.blobs.get(&text).expect("read")).expect("utf-8"),
        format!("The body of note {uid}.\r\n")
    );
    assert!(blobs.html.is_none(), "this message has no HTML alternative");
}

#[tokio::test]
async fn a_message_deleted_before_its_body_arrived_is_gone_rather_than_failed() {
    let backend = server(1).await;
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];
    MessageRepository::new(&local.connection)
        .delete(&[id])
        .expect("delete");

    let outcome = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        matches!(outcome, Outcome::Gone),
        "a body nobody has a row for is settled, not retried forever"
    );
}

#[tokio::test]
async fn a_dropped_connection_mid_body_stores_nothing() {
    let backend = server(1).await;
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    backend.inject(Fault::Disconnect);
    let error = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &CancelToken::new(),
    )
    .await
    .expect_err("the fetch died");
    assert!(matches!(error, postio_sync::SyncError::Backend(_)));

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert_eq!(
        stored.sync.body_state,
        BodyState::HeadersOnly,
        "half a body must never be recorded as a whole one"
    );
    assert!(stored.raw_blob_id.is_none());
}

#[tokio::test]
async fn a_cancelled_fetch_stores_nothing() {
    let backend = server(1).await;
    backend.set_latency(Duration::from_millis(20));
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    let cancel = CancelToken::new();
    cancel.cancel();
    let error = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &cancel,
    )
    .await
    .expect_err("cancelled");
    assert!(matches!(error, postio_sync::SyncError::Backend(_)));

    assert_eq!(
        MessageRepository::new(&local.connection)
            .get(id)
            .expect("get")
            .expect("row")
            .sync
            .body_state,
        BodyState::HeadersOnly
    );
    let _ = &local.database;
}

// ---------------------------------------------------------------------------
// And it reaches the search index
// ---------------------------------------------------------------------------

/// The indexed body text for `id`, or `None` if it has no shadow row at all.
fn indexed_body(connection: &postio_storage::PooledConnection, id: MessageId) -> Option<String> {
    connection
        .query_row(
            "SELECT body FROM search_documents WHERE message_id = ?1",
            [id.get()],
            |row| row.get::<_, String>(0),
        )
        .ok()
}

/// Issue #327: `index_body` existed, was tested, and nothing ever called it.
///
/// The metadata columns — sender, recipients, subject, filenames — are kept
/// current by SQL triggers, so they were always right. `body` is the one
/// column no trigger can compute: the text lives in the blob store, and the
/// only moment anything holds it is the moment it is fetched. Nothing did
/// the call, so `body` was empty on every message ever synced and search
/// answered a question about a subject and the same question about a body
/// differently — which is what "search is inconsistent" turned out to mean.
#[tokio::test]
async fn a_fetched_body_becomes_searchable_text() {
    let backend = server(2).await;
    let local = local();
    postio_index::index::ensure_schema(&local.connection).expect("the search schema");
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[1];

    assert_eq!(
        indexed_body(&local.connection, id).as_deref(),
        Some(""),
        "the header pass leaves the body column empty, which is the state \
         this test is about leaving behind"
    );

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert_eq!(
        indexed_body(&local.connection, id).as_deref(),
        Some(format!("The body of note {uid}.\r\n").as_str()),
        "the body landed in the blob store and never reached the index, so \
         a word that appears only in a message's body finds nothing (#327)"
    );
}

/// The other half of the same call: an HTML-only message is indexed as its
/// *text*, never as its markup.
///
/// Marketing mail, calendar invitations and anything composed in a webmail
/// client have no `text/plain` alternative at all, so this is the ordinary
/// case rather than an exotic one. Putting the markup in would make every
/// such message a hit for `div`, for `href`, and for the host of every
/// tracking redirect it carries — none of which the message says.
#[tokio::test]
async fn an_html_only_body_is_indexed_as_text_and_not_as_markup() {
    let raw = b"From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Quarterly\r\n\
         Message-ID: <html-1@example.com>\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         \r\n\
         <div><p>Turbines <a href=\"https://tracker.example/click?id=7\">stayed \
         nominal</a> all quarter.</p></div>\r\n"
        .to_vec();
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(MockMessage::new(raw).with_internal_date(at(1)));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    postio_index::index::ensure_schema(&local.connection).expect("the search schema");
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 4_096),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let indexed = indexed_body(&local.connection, id).expect("a shadow row");
    assert!(
        indexed.contains("stayed nominal"),
        "an HTML-only message is not findable by anything it actually says: \
         indexed as {indexed:?}"
    );
    for markup in ["div", "href", "tracker.example"] {
        assert!(
            !indexed.contains(markup),
            "{markup:?} is in the index, so this message is a hit for a word \
             it never contained: {indexed:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The text axis — ADR 0017
// ---------------------------------------------------------------------------

/// A message whose words are a rounding error beside its attachment.
///
/// The shape ADR 0017 is about: on the reference account, messages like this
/// are 15% of the mail and 90% of the bytes.
const HUGE: u64 = 40 * 1024 * 1024;

fn with_a_big_attachment(uid: u32) -> MockMessage {
    let structure = postio_imap::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_imap::backend::PartNode::new("1", "text/plain", 26)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_imap::backend::PartNode::new("2", "application/pdf", HUGE)
                .with_filename("statement.pdf"),
        ],
    );
    // The raw bytes exist so the mock can sketch an envelope, and are what a
    // whole-message fetch would return -- the test asserts nothing pulls them.
    MockMessage::new(
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             Subject: Statement {uid}\r\n\
             Message-ID: <statement-{uid}@example.com>\r\n\
             Content-Type: multipart/mixed; boundary=b\r\n\
             \r\n\
             --b\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             \r\n\
             Your statement is attached.\r\n\
             --b--\r\n"
        )
        .into_bytes(),
    )
    .with_internal_date(at(uid as i64))
    .with_structure(structure)
    .with_part("1", &b"Your statement is attached."[..])
}

#[tokio::test]
async fn backfilling_a_message_fetches_its_text_and_leaves_the_attachment_alone() {
    // The whole point of ADR 0017. Before it, this fetched `BODY.PEEK[]` --
    // forty megabytes across the wire to index twenty-six bytes of words.
    //
    // The mock rejects a `BODY[<section>]` nobody seeded, and only section 1
    // is seeded here, so a fetch that reached for the attachment fails the
    // test rather than quietly costing bandwidth no assertion can see.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_big_attachment(1));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    let outcome = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, HUGE),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        matches!(outcome, Outcome::Stored { bytes } if bytes < 1_024),
        "the text axis moves the words, not the payload: {outcome:?}"
    );

    let messages = MessageRepository::new(&local.connection);
    let stored = messages.get(id).expect("get").expect("row");

    let blobs = messages.body_blobs(id).expect("blobs").expect("the row");
    let text = blobs.text.expect("a text/plain body");
    assert_eq!(
        String::from_utf8(local.blobs.get(&text).expect("read")).expect("utf-8"),
        "Your statement is attached."
    );

    assert!(
        stored.raw_blob_id.is_none(),
        "the raw message is not stored: it is the forty megabytes this exists to avoid"
    );
    assert_eq!(
        stored.sync.body_state,
        BodyState::Partial,
        "text local, payload not -- the state migration 0001 declared and \
         nothing has ever written"
    );
}

#[tokio::test]
async fn a_message_with_no_attachments_is_full_once_its_text_is_local() {
    // `partial` has to mean something, so the other side of it needs proving:
    // a message whose every part is now local is `full`, and a reader can tell
    // the two apart without asking the network.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(
            MockMessage::new(note(1))
                .with_internal_date(at(1))
                .with_structure(postio_imap::backend::BodyStructure::from_parts(
                    "text/plain",
                    [postio_imap::backend::PartNode::new("1", "text/plain", 26)
                        .with_charset("utf-8")],
                ))
                .with_part("1", &b"The body of note 1.\r\n"[..]),
        );
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert_eq!(stored.sync.body_state, BodyState::Full);
}

#[tokio::test]
async fn a_row_synced_before_the_text_sections_existed_still_gets_its_body() {
    // Migration 0008 cannot invent sections for rows already on disk, and
    // guessing `1` would be wrong for every multipart message. Such a row
    // falls back to the whole-message fetch -- slower and fatter, but never
    // a message that silently has no body and never a hole in search.
    let backend = server(1).await;
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    // `server()` seeds no BODYSTRUCTURE, so the header pass records no
    // sections -- exactly the shape of a pre-0008 row.
    let messages = MessageRepository::new(&local.connection);
    assert_eq!(
        messages.get(id).expect("get").expect("row").text_part_id,
        None
    );

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = messages.get(id).expect("get").expect("row");
    let blobs = messages.body_blobs(id).expect("blobs").expect("the row");
    let text = blobs.text.expect("a text/plain body");
    assert_eq!(
        String::from_utf8(local.blobs.get(&text).expect("read")).expect("utf-8"),
        "The body of note 1.\r\n"
    );
    assert!(
        stored.raw_blob_id.is_some(),
        "the fallback fetched the whole message, so it keeps it"
    );
}

#[tokio::test]
async fn opening_an_attachment_asks_for_the_whole_message() {
    // The escape hatch from the text axis. The background lane leaves payloads
    // on the server, so when the user opens one there has to be a way to get
    // every byte -- otherwise ADR 0017 would have made attachments
    // unopenable, which is not a trade anyone agreed to.
    //
    // Interactive by construction: nothing speculative reaches this path.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_big_attachment(1));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    let mut request = request(&local.inbox, id, uid, HUGE);
    request.whole = true;

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert!(
        stored.raw_blob_id.is_some(),
        "every byte means every byte -- the raw message is what the part is cut from"
    );
    assert_eq!(stored.sync.body_state, BodyState::Full);
}

#[tokio::test]
async fn text_fetched_by_section_reaches_the_search_index() {
    // #327 was "bodies are never indexed", and the fix hung `index_body_of`
    // off the one place every body arrived. The text axis is a *second* place
    // bodies arrive, so it needs its own proof -- otherwise ADR 0017 would
    // quietly reintroduce the bug it exists to serve.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_big_attachment(1));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    postio_index::index::ensure_schema(&local.connection).expect("the search schema");
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, HUGE),
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let indexed: String = local
        .connection
        .query_row(
            "SELECT body FROM search_documents WHERE message_id = ?1",
            [id.get()],
            |row| row.get(0),
        )
        .expect("the shadow row");
    assert_eq!(indexed, "Your statement is attached.");
}
