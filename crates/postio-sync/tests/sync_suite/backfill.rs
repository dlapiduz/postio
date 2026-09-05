//! Backfilling message bodies: newest first, out of the user's way, and never
//! in front of the message they just opened.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{BodyState, Mailbox, MessageId, Uid, UidValidity};
use postio_storage::BlobStore;
use postio_storage::repository::{MailboxRepository, MessageRepository};
use postio_storage::test_support::{self, TempDatabase};
use postio_sync::backfill::{
    AttachmentPolicy, Backfill, BackfillPolicy, BodyRequest, Outcome, Priority, Want, fetch_body,
    request_body, request_payloads, seed, seed_header_blocks, seed_payloads,
};
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
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
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
        remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:{uid}")),
        size,
        received_at: at(uid as i64),
        want: Want::Text,
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
        .uids_in(local.inbox.id, postio_model::Generation::new(VALIDITY))
        .expect("uids")
        .into_iter()
        .map(|uid| {
            let message = messages
                .by_uid(local.inbox.id, postio_model::Generation::new(VALIDITY), uid)
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
// A folder can opt out of the background lane (ADR 0016, #350)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_excluded_folder_seeds_nothing_into_the_background_lane() {
    let backend = server(5).await;
    let local = local();
    headers(&local, &backend).await;

    MailboxRepository::new(&local.connection)
        .set_backfill_excluded(local.inbox.id, true)
        .expect("exclude the inbox");

    let mut backfill = Backfill::new(policy());
    let queued = seed(&local.connection, &mut backfill, local.inbox.id, 200).expect("seed");

    assert_eq!(
        queued, 0,
        "an excluded folder must not join the background lane"
    );
    assert!(backfill.next_body().is_none());
}

#[tokio::test]
async fn an_ordinary_folder_still_seeds_once_excluded_elsewhere() {
    // Regression guard beside the exclusion test above: the new check must
    // not turn into "nothing ever seeds again".
    let backend = server(5).await;
    let local = local();
    headers(&local, &backend).await;

    let mut backfill = Backfill::new(policy());
    let queued = seed(&local.connection, &mut backfill, local.inbox.id, 200).expect("seed");

    assert_eq!(queued, 5);
}

#[tokio::test]
async fn opening_a_message_in_an_excluded_folder_still_fetches_its_body() {
    // Turning off the background lane must not turn off reading: #350's own
    // acceptance criterion, and the same distinction
    // `BackfillPolicy::background`'s doc comment already draws for the
    // account-wide knob.
    let backend = server(1).await;
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    MailboxRepository::new(&local.connection)
        .set_backfill_excluded(local.inbox.id, true)
        .expect("exclude the inbox");

    let mut backfill = Backfill::new(policy());
    let asked = request_body(&local.connection, &mut backfill, id).expect("request_body");
    assert!(asked, "an on-open request must still be honoured");

    let outcome = fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");
    assert!(matches!(outcome, Outcome::Stored { .. }));
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
        BackfillPolicy::default().max_inline_bytes,
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

    let body = messages.body(id).expect("body").expect("the row");
    let text = body.text.expect("a text/plain body");
    assert_eq!(text, format!("The body of note {uid}.\r\n"));
    assert!(body.html.is_none(), "this message has no HTML alternative");
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
        BackfillPolicy::default().max_inline_bytes,
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
        BackfillPolicy::default().max_inline_bytes,
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
        BackfillPolicy::default().max_inline_bytes,
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

/// Whether `id`'s body has been indexed at all.
///
/// `message_bodies_fts` is `content = ''` — contentless — so unlike the
/// `search_documents.body` column these tests used to read, there is no text
/// to read back: the index keeps what it needs to answer a query and nothing
/// else (ADR 0016, and the reason it exists is that the column was the whole
/// mailbox duplicated inside SQLite). Presence and matching are therefore the
/// only two questions available, and between them they are the ones these
/// tests were always really asking.
fn body_is_indexed(connection: &postio_storage::PooledConnection, id: MessageId) -> bool {
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM message_bodies_fts WHERE rowid = ?1)",
            [id.get()],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

/// Whether `id`'s indexed body matches `query` — an FTS5 match expression, so
/// a bare word or a `"quoted phrase"`.
fn body_matches(connection: &postio_storage::PooledConnection, id: MessageId, query: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM message_bodies_fts
                             WHERE rowid = ?1 AND message_bodies_fts MATCH ?2)",
            rusqlite::params![id.get(), query],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
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

    assert!(
        !body_is_indexed(&local.connection, id),
        "the header pass indexes no body — it has none to index — which is \
         the state this test is about leaving behind"
    );

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1_024),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        body_matches(&local.connection, id, &format!("\"body of note {uid}\"")),
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
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        body_is_indexed(&local.connection, id),
        "an HTML-only message reached the index at all"
    );
    assert!(
        body_matches(&local.connection, id, "\"stayed nominal\""),
        "an HTML-only message is not findable by anything it actually says"
    );
    for markup in ["div", "href", "tracker.example"] {
        assert!(
            !body_matches(&local.connection, id, markup),
            "{markup:?} matches, so this message is a hit for a word it never \
             contained — the markup went into the index instead of the text"
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
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "text/plain", 26)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "application/pdf", HUGE)
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
        BackfillPolicy::default().max_inline_bytes,
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

    let body = messages.body(id).expect("body").expect("the row");
    let text = body.text.expect("a text/plain body");
    assert_eq!(text, "Your statement is attached.");

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
                .with_structure(postio_account::backend::BodyStructure::from_parts(
                    "text/plain",
                    [
                        postio_account::backend::PartNode::new("1", "text/plain", 26)
                            .with_charset("utf-8"),
                    ],
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
        BackfillPolicy::default().max_inline_bytes,
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
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = messages.get(id).expect("get").expect("row");
    let body = messages.body(id).expect("body").expect("the row");
    let text = body.text.expect("a text/plain body");
    assert_eq!(text, "The body of note 1.\r\n");
    assert!(
        stored.raw_blob_id.is_some(),
        "the fallback fetched the whole message, so it keeps it"
    );
}

#[tokio::test]
async fn a_payload_with_nothing_to_explain_its_bytes_asks_for_every_byte() {
    // The fallback under the payload axis. `BODY[2]` comes back encoded with
    // no headers, so a part whose `BODYSTRUCTURE` was never recorded -- a row
    // synced before migration 0010 -- has nothing to decode against. Fetching
    // the whole message is slower and fatter, and it is the only answer that
    // is not "store base64 and call it a PDF".
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
    request.want = Want::Whole;

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request,
        BackfillPolicy::default().max_inline_bytes,
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
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        body_matches(&local.connection, id, "\"statement is attached\""),
        "the text part's words did not reach the index"
    );
}

/// Whether `id` carries an indexed header row for `name`, whose value
/// contains `value` — the two questions `header:` asks, straight off the
/// table (ADR 0025 Q2).
fn header_is_indexed(
    connection: &postio_storage::PooledConnection,
    id: MessageId,
    name: &str,
    value: &str,
) -> bool {
    connection
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM message_headers
                             WHERE message_id = ?1 AND name = ?2
                               AND value LIKE '%' || ?3 || '%')",
            rusqlite::params![id.get(), name, value],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false)
}

#[tokio::test]
async fn a_fetched_body_reaches_the_header_index() {
    // #327's shape, in ADR 0025's costume: the catch-up pass is what fills
    // `message_headers` for mail that was already here, and it runs once per
    // start. If the fetch path did not index too, mail that arrived while the
    // application was open would not answer `header:` until it was restarted
    // -- a feature that works on old mail and not on new is worse than one
    // that does not work.
    let backend = server(2).await;
    let local = local();
    postio_index::index::ensure_schema(&local.connection).expect("the search schema");
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[1];

    assert!(
        !header_is_indexed(&local.connection, id, "content-type", "text/plain"),
        "the header-sync pass indexes no block -- it has none -- which is the \
         state this test is about leaving behind"
    );

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, 1024),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        header_is_indexed(&local.connection, id, "content-type", "text/plain"),
        "a fetched message's own headers did not reach the index -- and \
         `Content-Type` is a field no envelope column carries, so nothing but \
         the block could have answered it"
    );
}

#[tokio::test]
async fn text_fetched_by_section_reaches_the_header_index() {
    // The text axis is a second place a block arrives, and ADR 0017 says it
    // is 15% of the reference mailbox. It needs its own proof for the same
    // reason `text_fetched_by_section_reaches_the_search_index` does.
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
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    assert!(
        header_is_indexed(&local.connection, id, "from", "ada@example.com"),
        "the section fetch stored a block that nothing indexed"
    );
}

#[tokio::test]
async fn text_that_is_not_part_one_is_still_found() {
    // The reason the section number is stored rather than assumed. A message
    // whose first part is the attachment -- a scanner, a mail-merge, anything
    // that puts the payload first -- keeps its words at `2.1`, and a backfill
    // that reached for `1` would index a PDF's base64 as if it were prose.
    //
    // Only `2.1` and `2.2` are seeded, so a fetch of part 1 fails the test
    // rather than silently costing the bandwidth this exists to save.
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "application/pdf", HUGE)
                .with_filename("scan.pdf"),
            postio_account::backend::PartNode::new("2.1", "text/plain", 20).with_charset("utf-8"),
            postio_account::backend::PartNode::new("2.2", "text/html", 40).with_charset("utf-8"),
        ],
    );
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(
            MockMessage::new(
                &b"From: Ada Lovelace <ada@example.com>\r\n\
                   Subject: Scan\r\n\
                   Content-Type: multipart/mixed; boundary=b\r\n\
                   \r\n\
                   body sketched for the envelope only\r\n"[..],
            )
            .with_internal_date(at(1))
            .with_structure(structure)
            .with_part("2.1", &b"Scan attached."[..])
            .with_part("2.2", &b"<p>Scan attached.</p>"[..]),
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
        &request(&local.inbox, id, uid, HUGE),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let messages = MessageRepository::new(&local.connection);
    let body = messages.body(id).expect("body").expect("the row");

    let text = body.text.expect("the plain-text part at 2.1");
    assert_eq!(text, "Scan attached.");
    let html = body.html.expect("the HTML alternative at 2.2");
    assert_eq!(html, "<p>Scan attached.</p>");
    assert_eq!(
        messages.get(id).expect("get").expect("row").sync.body_state,
        BodyState::Partial,
        "the scan itself stayed on the server"
    );
}

// ---------------------------------------------------------------------------
// The backlog is bounded — ADR 0017, axis 2
// ---------------------------------------------------------------------------

#[test]
fn the_background_backlog_stops_growing_at_its_cap() {
    // The backlog is an in-memory `BinaryHeap` of requests, each carrying an
    // owned mailbox path. It is fine at 200 per folder and it is not fine the
    // first time something re-seeds "the whole folder" -- 81,744 entries is
    // the reference account, and ADR 0017's rule is that no mailbox is ever
    // resident in this process.
    //
    // Refusing the overflow rather than evicting the oldest: the heap is
    // newest-first, so what would be evicted is the mail most likely to be
    // opened, and `seed` re-reads from storage anyway. `body_state` is the
    // durable record of what still needs fetching; the heap is only a window
    // onto it.
    let mut backfill = Backfill::new(BackfillPolicy {
        max_backlog: 3,
        ..policy()
    });
    let mailbox = Mailbox::new(postio_model::AccountId::new(1), "INBOX", None);

    let queued = (1..=10)
        .filter(|uid| backfill.enqueue(request(&mailbox, MessageId::new(*uid as i64), *uid, 100)))
        .count();

    assert_eq!(queued, 3, "the cap is a cap");
    assert_eq!(backfill.progress().pending, 3);
}

#[test]
fn the_user_is_never_refused_by_the_backlog_cap() {
    // The rule that decides every question in this module: the interactive
    // lane always wins. A full backlog is a statement about speculative work,
    // and the message someone just opened is not speculative -- refusing it
    // would be the cap turning into a bug wearing a policy's clothes.
    let mut backfill = Backfill::new(BackfillPolicy {
        max_backlog: 1,
        ..policy()
    });
    let mailbox = Mailbox::new(postio_model::AccountId::new(1), "INBOX", None);

    assert!(backfill.enqueue(request(&mailbox, MessageId::new(1), 1, 100)));
    assert!(!backfill.enqueue(request(&mailbox, MessageId::new(2), 2, 100)));

    backfill.request_now(request(&mailbox, MessageId::new(3), 3, 100));

    let next = backfill.next_body().expect("work");
    assert_eq!(
        next.request.message,
        MessageId::new(3),
        "the one the user opened, ahead of the backlog and past its cap"
    );
}

// ---------------------------------------------------------------------------
// The payload axis (ADR 0017, #377)
// ---------------------------------------------------------------------------

/// A small payload and the base64 the server would hand back for it.
///
/// Written out rather than encoded here on purpose: the point of the fixture
/// is that the bytes on the wire are *not* the bytes in the blob store, and a
/// test that encoded the expectation itself could not tell the two apart.
const PDF: &[u8] = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
const PDF_BASE64: &str = "JVBERi0xLjQKJeLjz9MK\r\n";

/// A message whose text is section 1 and whose payload is section 2, with
/// both seeded on the server the way a real one would be: the text plain, the
/// payload base64, and `BODYSTRUCTURE` saying so.
fn with_a_payload(uid: u32, filename: &str) -> MockMessage {
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "text/plain", 26)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "application/pdf", PDF.len() as u64)
                .with_encoding("base64")
                .with_filename(filename),
        ],
    );
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
    .with_part("2", PDF_BASE64.as_bytes())
}

/// Header-sync one message carrying a payload, then backfill its text, so the
/// row is left exactly where the payload axis picks it up: `partial`.
async fn a_partial_message(local: &Local, backend: &MockBackend) -> (MessageId, u32) {
    let rows = headers(local, backend).await;
    let (id, uid) = rows[0];
    fetch_body(
        &local.connection,
        &local.blobs,
        backend,
        &request(&local.inbox, id, uid, 4_096),
        BackfillPolicy::default().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("text");
    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert_eq!(
        stored.sync.body_state,
        BodyState::Partial,
        "the fixture has to start where the payload axis begins"
    );
    (id, uid)
}

#[test]
fn a_payload_is_fetched_when_it_is_opened_and_not_before() {
    assert_eq!(AttachmentPolicy::default(), AttachmentPolicy::OnOpen);
    assert_eq!(
        BackfillPolicy::default().attachments,
        AttachmentPolicy::OnOpen,
        "ADR 0017: ~90% of a mailbox by weight is payloads, and pulling them \
         by default is the cost the split exists to refuse"
    );
}

#[tokio::test]
async fn opening_an_attachment_fetches_the_part_and_records_where_it_landed() {
    // The column the schema has had since migration 0001 and the receive path
    // has never written. Before this, `Attachment::is_downloaded` was false
    // for every message that ever arrived from a server.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;

    let mut backfill = Backfill::new(policy());
    assert!(
        request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask"),
        "there is a payload on the server to ask for"
    );
    let claim = backfill.next_body().expect("a claim");
    assert_eq!(
        claim.priority,
        Priority::Interactive,
        "the user asked for these bytes by name and is watching a spinner"
    );

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("fetch");

    let messages = MessageRepository::new(&local.connection);
    let stored = messages.get(id).expect("get").expect("row");
    let part = &stored.attachments[0];
    assert!(part.is_downloaded(), "the chip can honestly say 'open' now");
    let blob = part.blob_id.clone().expect("a key");
    assert_eq!(
        local.blobs.get(&blob).expect("read"),
        PDF,
        "decoded, not the base64 that came off the wire"
    );
    assert!(
        stored.raw_blob_id.is_none(),
        "a payload fetch pulls the payload, not the message around it"
    );
    assert_eq!(
        stored.sync.body_state,
        BodyState::Full,
        "every part is local now"
    );
}

#[tokio::test]
async fn a_payload_already_on_this_machine_is_never_fetched_twice() {
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;

    let mut backfill = Backfill::new(policy());
    request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask");
    let claim = backfill.next_body().expect("a claim");
    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("fetch");
    let after_first = backend.calls();

    assert!(
        !request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask"),
        "the bytes are here; opening it again must not reach the network"
    );
    assert!(backfill.next_body().is_none());
    assert_eq!(backend.calls(), after_first);
}

#[tokio::test]
async fn an_evicted_payload_can_be_fetched_again() {
    // The promise the whole ceiling rests on (#862). `evict_to_fit` deletes
    // bytes a row still points at -- which is exactly what separates it from
    // garbage collection -- and it is only safe because those bytes are
    // refetchable. So the test is not "the file is gone": it is that a store
    // which has just lost an attachment to its ceiling asks for it again and
    // gets the same bytes back.
    //
    // Structurally that rests on what `forget` writes: `blob_id = NULL` and
    // `body_state = partial`, which together are precisely what the payload
    // axis reads as "not here yet". An eviction that cleared one and not the
    // other would leave the attachment unreachable for ever, and nothing
    // downstream could tell.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;
    let messages = MessageRepository::new(&local.connection);

    let mut backfill = Backfill::new(policy());
    request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask");
    let claim = backfill.next_body().expect("a claim");
    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("fetch");
    // As the engine does: a claim stays in flight until its outcome is
    // reported, and a message in flight has its next request held rather than
    // queued.
    backfill.finished(
        id,
        Outcome::Stored {
            bytes: PDF.len() as u64,
        },
    );
    let blob = messages.get(id).expect("get").expect("row").attachments[0]
        .blob_id
        .clone()
        .expect("the payload is here to begin with");

    // A ceiling of nothing at all: the smallest store that is over budget.
    let report = local
        .blobs
        .evict_to_fit(&local.connection, 0)
        .expect("evict");
    assert_eq!(report.removed, 1, "the payload was the only thing to take");
    assert!(!local.blobs.contains(&blob), "the bytes really are gone");

    let evicted = messages.get(id).expect("get").expect("row");
    assert_eq!(
        evicted.sync.body_state,
        BodyState::Partial,
        "the row stops claiming a part it no longer has"
    );

    // And the part is askable again, from the same call the attachment chip
    // makes when somebody presses download.
    assert!(
        request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask"),
        "an evicted payload is a payload the server still has: eviction is \
         not a delete from the user's point of view"
    );
    let claim = backfill.next_body().expect("a claim for the evicted part");
    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("refetch");

    let restored = messages.get(id).expect("get").expect("row");
    let back = restored.attachments[0]
        .blob_id
        .clone()
        .expect("the payload came back");
    assert_eq!(
        local.blobs.get(&back).expect("read"),
        PDF,
        "the same bytes, decoded the same way"
    );
    assert_eq!(
        restored.sync.body_state,
        BodyState::Full,
        "and the message is whole again"
    );
}

#[tokio::test]
async fn two_messages_carrying_the_same_file_share_one_blob() {
    // Dedup is worth real bytes: on the reference account 22,878 named
    // attachment parts collapse to 13,099 distinct. Content addressing gets
    // it for free -- provided the id is taken on the decoded payload and not
    // on the base64, which differs by line wrapping alone.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"))
        .message(with_a_payload(2, "a-different-name.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let rows = headers(&local, &backend).await;
    let messages = MessageRepository::new(&local.connection);
    let mut keys = Vec::new();

    for (id, uid) in rows {
        fetch_body(
            &local.connection,
            &local.blobs,
            &backend,
            &request(&local.inbox, id, uid, 4_096),
            BackfillPolicy::default().max_inline_bytes,
            &CancelToken::new(),
        )
        .await
        .expect("text");

        let mut backfill = Backfill::new(policy());
        request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask");
        let claim = backfill.next_body().expect("a claim");
        fetch_body(
            &local.connection,
            &local.blobs,
            &backend,
            &claim.request,
            BackfillPolicy::default().max_inline_bytes,
            &claim.cancel,
        )
        .await
        .expect("fetch");

        let stored = messages.get(id).expect("get").expect("row");
        keys.push(stored.attachments[0].blob_id.clone().expect("a key"));
    }

    assert_eq!(keys[0], keys[1], "the same bytes are the same blob");
}

#[tokio::test]
async fn never_leaves_a_payload_on_the_server_even_when_it_is_opened() {
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;

    let mut backfill = Backfill::new(BackfillPolicy {
        attachments: AttachmentPolicy::Never,
        ..policy()
    });

    assert!(
        !request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask"),
        "filename search and nothing more -- that is what `never` promises"
    );
    assert!(backfill.next_body().is_none());
}

#[tokio::test]
async fn eager_queues_the_payloads_the_text_lane_left_behind() {
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;

    let mut backfill = Backfill::new(BackfillPolicy {
        attachments: AttachmentPolicy::Eager,
        ..policy()
    });
    assert_eq!(
        seed_payloads(&local.connection, &mut backfill, local.inbox.id, 10).expect("seed"),
        1
    );

    let claim = backfill.next_body().expect("a claim");
    assert_eq!(claim.request.message, id);
    assert_eq!(
        claim.priority,
        Priority::Background,
        "nobody is watching a spinner for a speculative payload"
    );

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("fetch");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert!(stored.attachments[0].is_downloaded());
    assert_eq!(stored.sync.body_state, BodyState::Full);
    assert_eq!(
        seed_payloads(&local.connection, &mut backfill, local.inbox.id, 10).expect("seed"),
        0,
        "and there is nothing left for it to find"
    );
}

#[tokio::test]
async fn a_message_is_full_only_once_its_last_payload_is_local() {
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "text/plain", 26)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "application/pdf", PDF.len() as u64)
                .with_encoding("base64")
                .with_filename("first.pdf"),
            postio_account::backend::PartNode::new("3", "text/csv", 10).with_filename("rows.csv"),
        ],
    );
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(
            with_a_payload(1, "first.pdf")
                .with_structure(structure)
                .with_part("3", &b"name,size\n"[..]),
        );
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;
    let messages = MessageRepository::new(&local.connection);

    for (part, expected) in [("2", BodyState::Partial), ("3", BodyState::Full)] {
        let mut backfill = Backfill::new(policy());
        request_payloads(&local.connection, &mut backfill, id, &[part.to_owned()]).expect("ask");
        let claim = backfill.next_body().expect("a claim");
        fetch_body(
            &local.connection,
            &local.blobs,
            &backend,
            &claim.request,
            BackfillPolicy::default().max_inline_bytes,
            &claim.cancel,
        )
        .await
        .expect("fetch");
        assert_eq!(
            messages.get(id).expect("get").expect("row").sync.body_state,
            expected,
            "after fetching part {part}"
        );
    }
}

#[test]
fn a_part_asked_for_while_another_is_on_the_wire_is_not_lost() {
    // Two chips clicked in quick succession. The second request cannot join a
    // fetch already on the wire, and dropping it silently would leave the
    // second spinner turning until it timed out -- so it waits for the first
    // to settle and is offered again.
    let local = local();
    let mut backfill = Backfill::new(policy());

    let mut first = request(&local.inbox, MessageId::new(1), 1, 4_096);
    first.want = Want::Payloads(vec!["2".to_owned()]);
    let mut second = first.clone();
    second.want = Want::Payloads(vec!["3".to_owned()]);

    backfill.request_now(first);
    let claim = backfill.next_body().expect("a claim");
    backfill.request_now(second);
    assert!(
        backfill.next_body().is_none(),
        "one fetch at a time is what keeps the user off the back of the queue"
    );

    backfill.finished(claim.request.message, Outcome::Stored { bytes: 10 });

    let next = backfill.next_body().expect("the deferred request");
    assert_eq!(next.request.want, Want::Payloads(vec!["3".to_owned()]));
}

#[test]
fn a_part_asked_for_while_another_is_still_queued_joins_it() {
    let local = local();
    let mut backfill = Backfill::new(policy());

    let mut first = request(&local.inbox, MessageId::new(1), 1, 4_096);
    first.want = Want::Payloads(vec!["2".to_owned()]);
    let mut second = first.clone();
    second.want = Want::Payloads(vec!["3".to_owned()]);

    backfill.request_now(first);
    backfill.request_now(second);

    let claim = backfill.next_body().expect("a claim");
    assert_eq!(
        claim.request.want,
        Want::Payloads(vec!["2".to_owned(), "3".to_owned()]),
        "one round trip, both parts"
    );
    assert!(backfill.next_body().is_none());
}

// ---------------------------------------------------------------------------
// The disk budget stops the lane — ADR 0017, axis 3
// ---------------------------------------------------------------------------

#[test]
fn a_full_store_pauses_the_background_lane_rather_than_thrashing() {
    // Backfill fetches, eviction frees, backfill fetches the same bytes
    // again: a store at its ceiling with the lane still running is a loop
    // that burns a data plan to stay exactly as full as it was. The lane
    // stops instead, and #352 is the surface that says so.
    let mut backfill = Backfill::new(policy());
    let mailbox = Mailbox::new(postio_model::AccountId::new(1), "INBOX", None);

    backfill.set_disk_full(true);
    assert!(!backfill.enqueue(request(&mailbox, MessageId::new(1), 1, 100)));
    assert!(backfill.is_paused());

    backfill.set_disk_full(false);
    assert!(backfill.enqueue(request(&mailbox, MessageId::new(1), 1, 100)));
}

#[test]
fn a_full_store_never_refuses_the_user() {
    // Same rule as every other pause in this module: the interactive lane
    // wins. Someone opening a message is not speculative work, and a store
    // over its ceiling is a statement about speculative work.
    //
    // It is also the only way out: opening mail is what proves which blobs
    // are worth keeping, and a client that stopped serving reads when its
    // cache filled would be a client that had stopped working.
    let mut backfill = Backfill::new(policy());
    let mailbox = Mailbox::new(postio_model::AccountId::new(1), "INBOX", None);

    backfill.set_disk_full(true);
    backfill.request_now(request(&mailbox, MessageId::new(7), 7, 100));

    let next = backfill.next_body().expect("work");
    assert_eq!(next.request.message, MessageId::new(7));
}

// ---------------------------------------------------------------------------
// Inline parts ride with the text — ADR 0017, #751
// ---------------------------------------------------------------------------

/// Base64 for `PNGBYTES`, so the decoded bytes are recognisable in a failure.
const SMALL_INLINE: &str = "UE5HQllURVM=";

/// A `multipart/related` message: an HTML body, a small inline image it
/// references, and an inline image far too big for the text axis to carry.
///
/// Only sections `1` and `2` are seeded. The mock rejects a `BODY[<section>]`
/// nobody seeded, so a fetch that reached for the oversized part fails the
/// test rather than quietly costing forty megabytes no assertion can see.
fn with_inline_images(uid: u32) -> MockMessage {
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/related",
        [
            postio_account::backend::PartNode::new("1", "text/html", 64)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "image/png", 8)
                .with_encoding("base64")
                // Brackets and all, exactly as `BODYSTRUCTURE` reports it.
                .with_content_id("<logo@example.com>")
                .with_disposition(postio_account::backend::Disposition::Inline),
            postio_account::backend::PartNode::new("3", "image/png", HUGE)
                .with_encoding("base64")
                .with_content_id("<banner@example.com>")
                .with_disposition(postio_account::backend::Disposition::Inline),
        ],
    );
    MockMessage::new(
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             Subject: Two inline shots {uid}\r\n\
             Message-ID: <inline-{uid}@example.com>\r\n\
             Content-Type: multipart/related; boundary=rel\r\n\
             \r\n\
             --rel\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             \r\n\
             <p><img src=\"cid:logo@example.com\"></p>\r\n\
             --rel--\r\n"
        )
        .into_bytes(),
    )
    .with_internal_date(at(uid as i64))
    .with_structure(structure)
    .with_part("1", &b"<p><img src=\"cid:logo@example.com\"></p>"[..])
    .with_part("2", SMALL_INLINE.as_bytes())
}

#[tokio::test]
async fn the_text_axis_carries_the_inline_images_the_body_references() {
    // #751 cause 1, and ADR 0017's "inline parts ride with the text": the rule
    // was decided, documented, and built by nothing, so `cid:` resolved to no
    // bytes and every HTML message with its own images drew broken boxes.
    //
    // Under the default `AttachmentPolicy::OnOpen` -- payloads stay on the
    // server until somebody asks -- the inline part under the cap must still
    // be local when the text lands, because it *is* the text as far as a
    // person reading the message is concerned.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_inline_images(1));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, HUGE),
        policy().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");

    let logo = stored
        .attachments
        .iter()
        .find(|part| part.content_id.as_deref() == Some("logo@example.com"))
        .expect("the inline part the HTML references, stored bare");
    let blob = logo.blob_id.clone().expect(
        "an inline part under the cap is fetched with the text, or the pane draws a broken box",
    );
    assert_eq!(
        local.blobs.get(&blob).expect("the blob"),
        b"PNGBYTES",
        "the inline part is stored decoded, as the payload axis stores one"
    );

    let banner = stored
        .attachments
        .iter()
        .find(|part| part.content_id.as_deref() == Some("banner@example.com"))
        .expect("the oversized inline part is still described");
    assert!(
        banner.blob_id.is_none(),
        "an inline part over the cap is a payload: it stays on the server \
         until somebody asks for it, which is what stops HTML mail dragging \
         an embedded video down the text axis"
    );

    assert_eq!(
        stored.sync.body_state,
        BodyState::Partial,
        "the words and the small image are local; the big one is not"
    );
}

#[tokio::test]
async fn a_message_whose_inline_parts_all_fit_is_full_once_its_text_lands() {
    // The other side of the cap: when nothing was left on the server, the
    // message is `full`, so the reader offers "open" rather than "download"
    // and search knows it is answering from a complete corpus.
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/related",
        [
            postio_account::backend::PartNode::new("1", "text/html", 64).with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "image/png", 8)
                .with_encoding("base64")
                .with_content_id("<logo@example.com>")
                .with_disposition(postio_account::backend::Disposition::Inline),
        ],
    );
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(
            MockMessage::new(note(1))
                .with_internal_date(at(1))
                .with_structure(structure)
                .with_part("1", &b"<p>Hello.</p>"[..])
                .with_part("2", SMALL_INLINE.as_bytes()),
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
        &request(&local.inbox, id, uid, 4_096),
        policy().max_inline_bytes,
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
async fn a_named_attachment_is_never_dragged_down_the_text_axis() {
    // The cap is about *inline* parts. A small PDF the sender attached is a
    // payload however little it weighs, or `attachment_fetch = "on_open"`
    // would quietly stop meaning anything. The mock has no bytes seeded for
    // section 2, so reaching for it fails this test.
    let structure = postio_account::backend::BodyStructure::from_parts(
        "multipart/mixed",
        [
            postio_account::backend::PartNode::new("1", "text/plain", 26)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            postio_account::backend::PartNode::new("2", "application/pdf", 512)
                .with_encoding("base64")
                .with_filename("receipt.pdf"),
        ],
    );
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(
            MockMessage::new(note(1))
                .with_internal_date(at(1))
                .with_structure(structure)
                .with_part("1", &b"Your receipt is attached."[..]),
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
        &request(&local.inbox, id, uid, 4_096),
        policy().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = MessageRepository::new(&local.connection)
        .get(id)
        .expect("get")
        .expect("row");
    assert!(
        stored.attachments.iter().all(|part| part.blob_id.is_none()),
        "a named attachment stays on the server under the default policy"
    );
    assert_eq!(stored.sync.body_state, BodyState::Partial);
}

// ---------------------------------------------------------------------------
// The header block — ADR 0025, #884
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_fetched_body_stores_the_header_block_it_arrived_with() {
    // `header:` has nowhere to match against until this happens. The column
    // has existed since migration 0001 and both backfill paths passed
    // `headers: None` on purpose -- "a copy nobody reads is a copy that can go
    // stale" -- which was right until ADR 0025 gave it a reader.
    let backend = server(1).await;
    let local = local();
    let rows = headers(&local, &backend).await;
    let (id, uid) = rows[0];

    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &request(&local.inbox, id, uid, HUGE),
        policy().max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("fetch");

    let stored = MessageRepository::new(&local.connection)
        .body(id)
        .expect("body")
        .expect("the row");
    let block = stored
        .headers
        .expect("the block the message arrived with, or header: has nothing to match");
    assert!(block.contains("Subject: Note 1"), "got: {block:?}");
    assert!(
        !block.contains("The body of note 1"),
        "the body is not a header and must not reach the index: {block:?}"
    );
    assert!(!stored.headers_truncated, "a note is not pathological");
}

#[tokio::test]
async fn the_text_axis_stores_a_block_even_though_it_stores_no_raw_blob() {
    // The `partial` path -- ~15% of ADR 0017's reference mailbox. It asks for
    // the text sections by name and stores no raw source at all, so it is the
    // one state where nothing else would ever bring the block and `header:`
    // would answer "no such mail" for mail that is right there. It now asks
    // for `BODY[HEADER]` as well, which is one more section on a fetch that is
    // already happening.
    //
    // The payload fetch below is not what stores it -- `fetch_payloads` writes
    // no body at all -- and this test said otherwise until the block was
    // checked against which function actually wrote it. It is here to prove
    // the block survives the message going from `partial` to `full`.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;

    let mut backfill = Backfill::new(policy());
    request_payloads(&local.connection, &mut backfill, id, &["2".to_owned()]).expect("ask");
    let claim = backfill.next_body().expect("a claim");
    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("fetch");

    let messages = MessageRepository::new(&local.connection);
    let stored = messages.body(id).expect("body").expect("the row");
    let block = stored
        .headers
        .expect("a payload fetch has to bring the block, because nothing else will");
    assert!(
        block.contains("statement.pdf") || block.contains("Subject"),
        "got: {block:?}"
    );
    assert!(
        messages
            .get(id)
            .expect("get")
            .expect("row")
            .raw_blob_id
            .is_none(),
        "this path stores no raw blob, which is exactly why it must store the block"
    );
}

#[tokio::test]
async fn a_legacy_row_with_no_block_and_no_blob_is_queued_and_filled() {
    // The third row of #884's repair table, and the only one that touches the
    // network. A store that has been in use since before blocks were stored
    // has messages whose body is local, whose raw source was never kept -- the
    // `partial` path never keeps one -- and whose block therefore cannot be
    // rebuilt from disk. Without this they would stay unanswerable for ever,
    // which is the shape of "the feature works only on mail you received after
    // you upgraded".
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(with_a_payload(1, "statement.pdf"));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let local = local();
    let (id, _uid) = a_partial_message(&local, &backend).await;
    let messages = MessageRepository::new(&local.connection);

    // Wind it back to what a pre-#884 store holds: body local, block absent.
    messages.set_headers(id, None).expect("clear the block");
    assert!(
        messages
            .body(id)
            .expect("body")
            .expect("row")
            .headers
            .is_none(),
        "the fixture has to start where a legacy store is"
    );
    assert!(
        messages
            .get(id)
            .expect("get")
            .expect("row")
            .raw_blob_id
            .is_none(),
        "and with no raw source to rebuild it from"
    );

    let mut backfill = Backfill::new(policy());
    let queued =
        seed_header_blocks(&local.connection, &mut backfill, local.inbox.id, 10).expect("seed");
    assert_eq!(
        queued, 1,
        "the row needs a block and nothing local can give it"
    );

    let claim = backfill.next_body().expect("a claim");
    assert_eq!(claim.request.want, Want::HeaderBlock);
    fetch_body(
        &local.connection,
        &local.blobs,
        &backend,
        &claim.request,
        BackfillPolicy::default().max_inline_bytes,
        &claim.cancel,
    )
    .await
    .expect("fetch");

    let stored = messages.body(id).expect("body").expect("row");
    assert!(
        stored
            .headers
            .as_deref()
            .is_some_and(|block| block.contains("Subject")),
        "got: {:?}",
        stored.headers
    );
    assert_eq!(
        stored.text.as_deref(),
        Some("Your statement is attached."),
        "a header fetch must not disturb the words already on this machine"
    );

    // And it stops being offered, or the lane spins (#500).
    assert_eq!(
        seed_header_blocks(&local.connection, &mut backfill, local.inbox.id, 10).expect("seed"),
        0
    );
}
