//! The message repository, and the windowed paging query the list depends on.
//!
//! Written before the repository existed. The bead's acceptance criteria are
//! "paging over a 100k-message fixture stays flat in time and memory", "the
//! sort key is stable across inserts (no row skipping while paging)" and a
//! recorded benchmark; the first two have tests here, and
//! `the_message_list_plan_never_sorts` is the structural half of the first.

use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;

use postio_model::{
    Attachment, BodyState, Disposition, EmailAddress, Flag, FlagSet, MailboxId, Message, MessageId,
    ModSeq, RfcMessageId, ThreadId, Uid, UidValidity,
};
use postio_storage::repository::{BodyBlobs, FlagSource, ListCursor, ListQuery, MessageRepository};
use postio_storage::test_support;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

/// A message with enough on it to be worth round-tripping.
fn a_message(mailbox: MailboxId, account: postio_model::AccountId, seconds: i64) -> Message {
    let mut message = Message::new(account, mailbox, at(seconds));
    message.rfc_message_id = Some(RfcMessageId::new(format!("<m{seconds}@example.com>")));
    message.in_reply_to = Some(RfcMessageId::new("<parent@example.com>"));
    message.references = vec![
        RfcMessageId::new("<root@example.com>"),
        RfcMessageId::new("<parent@example.com>"),
    ];
    message.from = vec![EmailAddress::new(Some("Ada Norwood"), "ada@example.com")];
    message.to = vec![
        EmailAddress::new(Some("Quinn Abara"), "quinn@example.net"),
        EmailAddress::new(None::<String>, "list@example.org"),
    ];
    message.cc = vec![EmailAddress::new(None::<String>, "cc@example.com")];
    message.subject = Some(format!("Re: Subject {seconds}"));
    message.date = Some(at(seconds - 10));
    message.preview = Some("A short snippet".to_owned());
    message.size = 4_096;
    message.flags = [Flag::Seen, Flag::Keyword("Work".to_owned())]
        .into_iter()
        .collect();
    message.server.uid = Some(Uid::new(seconds as u32 + 1));
    message.server.uid_validity = Some(UidValidity::new(99));
    message.server.mod_seq = Some(ModSeq::new(12_345));
    message.sync.body_state = BodyState::HeadersOnly;
    message
}

// ---------------------------------------------------------------------------
// Create, read, update, delete
// ---------------------------------------------------------------------------

#[test]
fn a_message_round_trips_with_its_recipients_and_attachments() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 100);
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 2_048);
    attachment.filename = Some("layout.pdf".to_owned());
    attachment.part_id = Some("2".to_owned());
    attachment.disposition = Disposition::Attachment;
    let mut inline = Attachment::new(MessageId::UNASSIGNED, "image/png", 512);
    inline.disposition = Disposition::Inline;
    inline.content_id = Some("logo@example.com".to_owned());
    message.attachments = vec![attachment, inline];

    let id = messages.create(&mut message).expect("create");

    assert!(id.is_assigned());
    for attachment in &message.attachments {
        assert!(attachment.id.is_assigned(), "attachments get ids too");
        assert_eq!(attachment.message_id, id);
    }

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.rfc_message_id, message.rfc_message_id);
    assert_eq!(stored.in_reply_to, message.in_reply_to);
    assert_eq!(stored.references, message.references);
    assert_eq!(stored.from, message.from);
    assert_eq!(stored.to, message.to, "recipient order is header order");
    assert_eq!(stored.cc, message.cc);
    assert_eq!(stored.subject, message.subject);
    assert_eq!(stored.date, message.date);
    assert_eq!(stored.received_at, message.received_at);
    assert_eq!(stored.preview, message.preview);
    assert_eq!(stored.size, 4_096);
    assert_eq!(stored.flags, message.flags);
    assert_eq!(stored.attachments, message.attachments);
    assert!(stored.has_attachments());
    assert_eq!(stored.server, message.server);
    assert_eq!(stored.sync.body_state, BodyState::HeadersOnly);
}

#[test]
fn a_message_s_own_content_type_round_trips() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 200);
    message.content_type = Some("multipart/related".to_owned());
    let id = messages.create(&mut message).expect("create");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.content_type.as_deref(), Some("multipart/related"));
}

#[test]
fn a_message_with_no_content_type_recorded_reads_back_as_none() {
    // A row synced before this field existed, or a draft nothing has parsed
    // `BODYSTRUCTURE` for yet -- distinct from an empty string, which would
    // be a wrong answer rather than an honest "not known".
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 201);
    assert_eq!(message.content_type, None);
    let id = messages.create(&mut message).expect("create");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.content_type, None);
}

#[test]
fn a_message_s_list_id_round_trips() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 300);
    message.list_id = Some("harbour-dev.lists.example.org".to_owned());
    let id = messages.create(&mut message).expect("create");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(
        stored.list_id.as_deref(),
        Some("harbour-dev.lists.example.org")
    );
}

#[test]
fn a_message_with_no_list_id_reads_back_as_none() {
    // Most mail is not list mail; a `None` here must stay `None`, not an
    // empty string that would misread as a list with no name.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 301);
    assert_eq!(message.list_id, None);
    let id = messages.create(&mut message).expect("create");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.list_id, None);
}

#[test]
fn flags_are_denormalized_so_the_list_never_parses_a_string() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 1);
    message.flags = [Flag::Seen, Flag::Flagged, Flag::Answered, Flag::Recent]
        .into_iter()
        .collect();
    let id = messages.create(&mut message).expect("create");

    let (flags, seen, flagged, answered, draft): (String, bool, bool, bool, bool) = connection
        .query_row(
            "SELECT flags, seen, flagged, answered, draft FROM messages WHERE id = ?1",
            [id.get()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read the raw row");

    assert!(seen && flagged && answered && !draft);
    assert!(
        !flags.contains("Recent"),
        "\\Recent is per-session and must never be persisted: {flags:?}"
    );
    assert_eq!(flags, "\\Seen \\Answered \\Flagged", "canonical order");
    assert!(
        !messages
            .get(id)
            .expect("get")
            .expect("the message")
            .flags
            .contains(&Flag::Recent)
    );
}

#[test]
fn updating_a_message_replaces_its_recipients_rather_than_appending() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 2);
    let id = messages.create(&mut message).expect("create");

    message.to = vec![EmailAddress::new(None::<String>, "only@example.com")];
    message.subject = Some("Rewritten".to_owned());
    message.attachments.clear();
    messages.update(&mut message).expect("update");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.to.len(), 1);
    assert_eq!(stored.subject.as_deref(), Some("Rewritten"));

    let recipients: i64 = connection
        .query_row("SELECT count(*) FROM recipients", [], |row| row.get(0))
        .expect("count");
    assert_eq!(recipients, 3, "from + to + cc, with no leftovers");
}

#[test]
fn deleting_messages_takes_their_recipients_and_attachments() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut first = a_message(inbox, account.id, 3);
    let mut second = a_message(inbox, account.id, 4);
    let first_id = messages.create(&mut first).expect("create");
    let second_id = messages.create(&mut second).expect("create");

    assert_eq!(messages.delete(&[first_id, second_id]).expect("delete"), 2);
    assert!(messages.get(first_id).expect("get").is_none());

    for table in ["messages", "recipients", "attachments"] {
        let remaining: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(remaining, 0, "{table}");
    }
    assert_eq!(
        messages.delete(&[first_id]).expect("delete again"),
        0,
        "deleting what is gone is zero, not an error"
    );
}

#[test]
fn the_body_and_headers_live_in_the_blob_store_and_the_row_holds_the_keys() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 5);
    message.raw_blob_id = Some(postio_model::BlobId::new("a".repeat(64)));
    let id = messages.create(&mut message).expect("create");

    assert_eq!(
        messages.body_blobs(id).expect("blobs").expect("the row"),
        BodyBlobs::default(),
        "nothing has been downloaded yet"
    );

    let blobs = BodyBlobs {
        text: Some(postio_model::BlobId::new("b".repeat(64))),
        html: Some(postio_model::BlobId::new("c".repeat(64))),
        headers: Some(postio_model::BlobId::new("d".repeat(64))),
    };
    messages
        .set_body_blobs(id, &blobs, BodyState::Full)
        .expect("set");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(messages.body_blobs(id).expect("blobs"), Some(blobs));
    assert_eq!(stored.raw_blob_id, message.raw_blob_id);
    assert_eq!(stored.sync.body_state, BodyState::Full);
    assert!(
        stored.body.is_empty() && stored.headers.is_empty(),
        "the bytes themselves are the blob store's, not SQLite's"
    );
}

// ---------------------------------------------------------------------------
// Backfill candidates
// ---------------------------------------------------------------------------

#[test]
fn needing_backfill_returns_newest_first_and_skips_full_bodies() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut oldest = a_message(inbox, account.id, 1);
    let mut newest = a_message(inbox, account.id, 3);
    let mut already_full = a_message(inbox, account.id, 2);
    already_full.sync.body_state = BodyState::Full;

    for message in [&mut oldest, &mut newest, &mut already_full] {
        messages.create(message).expect("create");
    }

    let candidates = messages.needing_backfill(inbox, 10).expect("query");
    assert_eq!(
        candidates.iter().map(|c| c.message_id).collect::<Vec<_>>(),
        vec![newest.id, oldest.id],
        "newest first, and the fully-fetched message is not a candidate"
    );
    assert_eq!(candidates[0].mailbox_path, "INBOX");
    assert_eq!(candidates[0].uid, newest.server.uid.unwrap());
    assert_eq!(candidates[0].size, newest.size);
    assert_eq!(candidates[0].received_at, newest.received_at);
}

#[test]
fn needing_backfill_is_windowed_and_scoped_to_its_mailbox() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive");
    let messages = MessageRepository::new(&connection);

    for seconds in 0..5 {
        messages
            .create(&mut a_message(inbox, account.id, seconds))
            .expect("create");
    }
    messages
        .create(&mut a_message(archive.id, account.id, 99))
        .expect("create in another mailbox");

    let limited = messages.needing_backfill(inbox, 2).expect("query");
    assert_eq!(limited.len(), 2, "the window caps how many come back");

    let archived = messages.needing_backfill(archive.id, 10).expect("query");
    assert_eq!(
        archived.len(),
        1,
        "a mailbox never sees another mailbox's candidates"
    );
}

#[test]
fn a_message_with_no_uid_yet_is_not_a_backfill_candidate() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut composed = a_message(inbox, account.id, 1);
    composed.server.uid = None;
    messages.create(&mut composed).expect("create");

    assert!(
        messages
            .needing_backfill(inbox, 10)
            .expect("query")
            .is_empty()
    );
    assert!(
        messages
            .backfill_candidate(composed.id)
            .expect("query")
            .is_none()
    );
}

#[test]
fn backfill_candidate_looks_up_a_single_message_by_id_alone() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 7);
    messages.create(&mut message).expect("create");

    let candidate = messages
        .backfill_candidate(message.id)
        .expect("query")
        .expect("a candidate, since the body is headers-only");
    assert_eq!(candidate.mailbox_id, inbox);
    assert_eq!(candidate.mailbox_path, "INBOX");
    assert_eq!(candidate.uid, message.server.uid.unwrap());

    messages
        .set_body_blobs(message.id, &BodyBlobs::default(), BodyState::Full)
        .expect("mark it fetched");
    assert!(
        messages
            .backfill_candidate(message.id)
            .expect("query")
            .is_none(),
        "a message that already has its full body is not a candidate"
    );
}

// ---------------------------------------------------------------------------
// Batch upsert, the shape sync writes in
// ---------------------------------------------------------------------------

#[test]
fn upserting_a_batch_inserts_what_is_new_and_updates_what_is_known() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![
        a_message(inbox, account.id, 10),
        a_message(inbox, account.id, 11),
    ];
    let report = messages.upsert_batch(&mut batch).expect("first upsert");
    assert_eq!(report.inserted, 2);
    assert_eq!(report.updated, 0);
    let ids: Vec<MessageId> = batch.iter().map(|message| message.id).collect();
    assert!(ids.iter().copied().all(MessageId::is_assigned));

    // The same UIDs come back with a flag change, plus one genuinely new one.
    let mut again = vec![
        a_message(inbox, account.id, 10),
        a_message(inbox, account.id, 11),
        a_message(inbox, account.id, 12),
    ];
    again[0].flags = [Flag::Seen, Flag::Flagged].into_iter().collect();
    let report = messages.upsert_batch(&mut again).expect("second upsert");

    assert_eq!(report.inserted, 1);
    assert_eq!(report.updated, 2);
    assert_eq!(
        again[0].id, ids[0],
        "a message keeps its local id across a resync, so the UI's selection survives"
    );

    let total: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("count");
    assert_eq!(total, 3, "no duplicates");
    assert!(
        messages
            .get(ids[0])
            .expect("get")
            .expect("the message")
            .flags
            .is_flagged()
    );
}

#[test]
fn a_locally_composed_message_with_no_uid_is_never_matched_by_upsert() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut first = a_message(inbox, account.id, 20);
    first.server = Default::default();
    let mut second = a_message(inbox, account.id, 21);
    second.server = Default::default();

    let mut batch = vec![first, second];
    let report = messages.upsert_batch(&mut batch).expect("upsert");

    assert_eq!(
        report.inserted, 2,
        "with no server identity there is nothing to match on"
    );
    assert_ne!(batch[0].id, batch[1].id);
}

// ---------------------------------------------------------------------------
// Lookups
// ---------------------------------------------------------------------------

#[test]
fn a_message_can_be_found_by_its_server_uid() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 30);
    let id = messages.create(&mut message).expect("create");
    let uid = message.server.uid.unwrap();
    let validity = message.server.uid_validity.unwrap();

    assert_eq!(
        messages
            .by_uid(inbox, validity, uid)
            .expect("by uid")
            .map(|message| message.id),
        Some(id)
    );
    assert!(
        messages
            .by_uid(inbox, UidValidity::new(100), uid)
            .expect("by uid")
            .is_none(),
        "a UID means nothing under a different UIDVALIDITY"
    );
    assert_eq!(messages.uids_in(inbox, validity).expect("uids"), vec![uid]);
    assert!(
        messages
            .uids_in(inbox, UidValidity::new(100))
            .expect("uids")
            .is_empty()
    );
}

#[test]
fn messages_can_be_found_by_rfc_message_id_and_duplicates_all_come_back() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let shared = RfcMessageId::new("<shared@example.com>");
    let mut first = a_message(inbox, account.id, 40);
    first.rfc_message_id = Some(shared.clone());
    let mut second = a_message(inbox, account.id, 41);
    second.rfc_message_id = Some(shared.clone());
    let first_id = messages.create(&mut first).expect("create");
    let second_id = messages.create(&mut second).expect("create");

    let found = messages
        .ids_by_rfc_message_id(account.id, &shared)
        .expect("lookup");

    assert_eq!(
        found,
        vec![first_id, second_id],
        "a Message-ID is not unique in the wild; the corpus has a fixture that reuses one"
    );
    assert!(
        messages
            .ids_by_rfc_message_id(account.id, &RfcMessageId::new("nothing@example.com"))
            .expect("lookup")
            .is_empty()
    );
    assert_eq!(
        messages
            .ids_by_rfc_message_id(account.id, &RfcMessageId::new("<SHARED@EXAMPLE.COM>"))
            .expect("lookup"),
        vec![first_id, second_id],
        "Message-IDs compare case-insensitively, the way threading needs"
    );
}

// ---------------------------------------------------------------------------
// Flags, moves, local delete
// ---------------------------------------------------------------------------

#[test]
fn a_local_flag_change_marks_the_row_dirty_and_a_server_one_does_not() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 50);
    let id = messages.create(&mut message).expect("create");

    let mut flags = FlagSet::new();
    flags.insert(Flag::Flagged);
    messages
        .set_flags(id, &flags, FlagSource::Local)
        .expect("local change");

    let stored = messages.get(id).expect("get").expect("the message");
    assert!(stored.flags.is_flagged() && !stored.flags.is_seen());
    assert!(
        stored.sync.flags_dirty,
        "a local change is ahead of the server until it is pushed"
    );

    messages
        .set_flags(id, &flags, FlagSource::Server)
        .expect("server change");
    assert!(
        !messages
            .get(id)
            .expect("get")
            .expect("the message")
            .sync
            .flags_dirty,
        "what the server told us is by definition not ahead of it"
    );
}

#[test]
fn moving_a_message_drops_the_uid_it_had_in_the_old_mailbox() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive");
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 60);
    let id = messages.create(&mut message).expect("create");

    assert_eq!(messages.move_to(&[id], archive.id).expect("move"), 1);

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.mailbox_id, archive.id);
    assert_eq!(
        stored.server.uid, None,
        "a UID belongs to the mailbox it was issued in"
    );
    assert_eq!(stored.server.uid_validity, None);
    assert!(
        stored.sync.has_pending_operations,
        "the move still has to be pushed"
    );
}

#[test]
fn a_locally_deleted_message_is_hidden_from_the_list_but_still_there() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 70);
    let id = messages.create(&mut message).expect("create");

    assert_eq!(messages.set_deleted_locally(&[id], true).expect("hide"), 1);
    assert!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .expect("page")
            .is_empty(),
        "the list hides it the instant the user presses the key"
    );
    assert!(
        messages.get(id).expect("get").is_some(),
        "but undo has to be able to bring it back"
    );

    messages.set_deleted_locally(&[id], false).expect("undo");
    assert_eq!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .expect("page")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// The windowed list query
// ---------------------------------------------------------------------------

/// Inserts `count` messages straight into the table, newest last.
///
/// Raw SQL and one statement: this is the fixture for the paging tests, and
/// building it through the repository would make them a test of insert speed.
fn seed(connection: &Connection, mailbox: MailboxId, count: u32) {
    connection
        .execute(
            "WITH RECURSIVE seq(n) AS (
                 SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?2
             )
             INSERT INTO messages (account_id, mailbox_id, received_at, subject, preview,
                                   flags, seen, flagged, size)
             SELECT (SELECT account_id FROM mailboxes WHERE id = ?1), ?1,
                    1770000000000 + n * 1000, 'Subject ' || n, 'Preview ' || n,
                    '', n % 2, n % 7 = 0, 1024
               FROM seq",
            rusqlite::params![mailbox.get(), count],
        )
        .expect("seed messages");
    connection
        .execute(
            "INSERT INTO recipients (message_id, kind, position, name, address,
                                     address_normalized)
             SELECT id, 'from', 0, 'Sender ' || id, 'sender' || id || '@example.com',
                    'sender' || id || '@example.com'
               FROM messages WHERE mailbox_id = ?1",
            [mailbox.get()],
        )
        .expect("seed senders");
}

#[test]
fn a_page_is_newest_first_and_no_longer_than_the_window() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection);
    seed(&connection, inbox, 10);
    let messages = MessageRepository::new(&connection);

    let page = messages
        .page(&ListQuery::mailbox(inbox).limit(4))
        .expect("page");

    assert_eq!(page.len(), 4);
    assert_eq!(page[0].subject.as_deref(), Some("Subject 10"));
    assert_eq!(page[3].subject.as_deref(), Some("Subject 7"));
    assert!(
        page[0].received_at > page[1].received_at,
        "newest first, always"
    );
    assert_eq!(
        page[0].from.as_ref().map(|from| from.display()),
        Some("Sender 10"),
        "the row carries its sender without a second query per row"
    );
    assert_eq!(page[0].preview.as_deref(), Some("Preview 10"));
    assert!(!page[0].seen, "message 10 is odd, so unread");
}

#[test]
fn paging_with_a_cursor_walks_the_whole_mailbox_exactly_once() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection);
    seed(&connection, inbox, 250);
    let messages = MessageRepository::new(&connection);

    let mut seen: Vec<MessageId> = Vec::new();
    let mut cursor: Option<ListCursor> = None;
    loop {
        let mut query = ListQuery::mailbox(inbox).limit(40);
        if let Some(cursor) = cursor {
            query = query.after(cursor);
        }
        let page = messages.page(&query).expect("page");
        if page.is_empty() {
            break;
        }
        cursor = page.last().map(|row| row.cursor());
        seen.extend(page.iter().map(|row| row.id));
    }

    assert_eq!(seen.len(), 250, "every message, once");
    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 250, "and none of them twice");
}

#[test]
fn a_message_arriving_mid_scroll_does_not_make_the_list_skip_a_row() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    seed(&connection, inbox, 100);
    let messages = MessageRepository::new(&connection);

    let first = messages
        .page(&ListQuery::mailbox(inbox).limit(10))
        .expect("first page");
    let cursor = first.last().expect("a row").cursor();

    // IDLE delivers a new message at the top while the user is still scrolling.
    let mut arrival = a_message(inbox, account.id, 1_000_000);
    messages.create(&mut arrival).expect("create");

    let second = messages
        .page(&ListQuery::mailbox(inbox).limit(10).after(cursor))
        .expect("second page");

    assert_eq!(
        second[0].subject.as_deref(),
        Some("Subject 90"),
        "the cursor is the sort key, so the next page continues where the last ended"
    );
    let overlap = first
        .iter()
        .any(|row| second.iter().any(|other| other.id == row.id));
    assert!(!overlap, "and nothing is shown twice");
}

#[test]
fn paging_by_offset_is_available_for_a_windowed_list_model() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection);
    seed(&connection, inbox, 100);
    let messages = MessageRepository::new(&connection);

    let page = messages
        .page_at(&ListQuery::mailbox(inbox).limit(5), 20)
        .expect("page at an offset");

    assert_eq!(page.len(), 5);
    assert_eq!(
        page[0].subject.as_deref(),
        Some("Subject 80"),
        "row 21 counting from the newest"
    );
    assert_eq!(
        messages.count(&ListQuery::mailbox(inbox)).expect("count"),
        100
    );
}

#[test]
fn the_list_can_be_scoped_to_an_account_or_to_flagged_messages() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive");
    seed(&connection, inbox, 10);
    seed(&connection, archive.id, 10);
    let messages = MessageRepository::new(&connection);

    assert_eq!(
        messages.count(&ListQuery::mailbox(inbox)).expect("count"),
        10
    );
    assert_eq!(
        messages
            .count(&ListQuery::account(account.id))
            .expect("count"),
        20,
        "the unified view spans mailboxes"
    );
    assert_eq!(
        messages
            .count(&ListQuery::flagged(account.id))
            .expect("count"),
        2,
        "every seventh message, in each of the two mailboxes"
    );
    assert!(
        messages
            .page(&ListQuery::flagged(account.id))
            .expect("page")
            .iter()
            .all(|row| row.flagged)
    );
}

#[test]
fn a_thread_id_travels_on_the_list_row_so_the_list_can_group_without_a_second_query() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 80);
    let id = messages.create(&mut message).expect("create");
    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1)",
            [account.id.get()],
        )
        .expect("a thread");
    messages
        .set_thread(id, Some(ThreadId::new(1)))
        .expect("assign");

    let page = messages.page(&ListQuery::mailbox(inbox)).expect("page");
    assert_eq!(page[0].thread_id, Some(ThreadId::new(1)));
    assert_eq!(
        messages
            .get(id)
            .expect("get")
            .expect("the message")
            .thread_id,
        Some(ThreadId::new(1))
    );
}

#[test]
fn a_thread_is_a_scope_of_its_own_so_a_drill_in_is_not_limited_to_one_folder() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive");
    let messages = MessageRepository::new(&connection);
    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1), (2, ?1)",
            [account.id.get()],
        )
        .expect("two threads");

    // A conversation half of which has been archived, which is what an
    // ordinary thread looks like after anyone has tidied up — plus a second
    // thread in the same folder, so a scope that ignored the thread entirely
    // would be caught rather than passing by luck.
    let mut ours = Vec::new();
    for (mailbox, minute) in [(inbox, 10), (archive.id, 20), (inbox, 30), (archive.id, 40)] {
        let mut message = a_message(mailbox, account.id, minute);
        let id = messages.create(&mut message).expect("create");
        messages
            .set_thread(id, Some(ThreadId::new(1)))
            .expect("assign");
        ours.push(id);
    }
    let mut other = a_message(inbox, account.id, 50);
    let elsewhere = messages.create(&mut other).expect("create");
    messages
        .set_thread(elsewhere, Some(ThreadId::new(2)))
        .expect("assign");

    let thread = ListQuery::thread(ThreadId::new(1));
    assert_eq!(
        messages.count(&thread).expect("count"),
        4,
        "the thread spans two folders and the scope has to span them too"
    );

    let page = messages.page(&thread).expect("page");
    let mut found: Vec<_> = page.iter().map(|row| row.id).collect();
    found.sort_by_key(|id| id.get());
    assert_eq!(found, ours, "every message of the thread, and only those");
    assert!(
        !page.iter().any(|row| row.id == elsewhere),
        "another thread in the same folder must not come along"
    );
}

// ---------------------------------------------------------------------------
// Acceptance: flat in time and memory over a large mailbox
// ---------------------------------------------------------------------------

/// Whether a plan resolves through an index rather than a scan or a sort.
fn plan_of(connection: &Connection, sql: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|error| panic!("prepare {sql}: {error}"));
    // The list query is parameterised; the planner does not care what the
    // values are, only that there are the right number of them.
    let arguments = vec![1i64; statement.parameter_count()];
    let rows = statement
        .query_map(rusqlite::params_from_iter(arguments), |row| {
            row.get::<_, String>(3)
        })
        .expect("plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    rows.join("\n")
}

#[test]
fn the_message_list_plan_never_sorts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    for (label, query) in [
        ("mailbox", ListQuery::mailbox(inbox)),
        (
            "account",
            ListQuery::account(postio_model::AccountId::new(1)),
        ),
        (
            "flagged",
            ListQuery::flagged(postio_model::AccountId::new(1)),
        ),
        ("thread", ListQuery::thread(ThreadId::new(1))),
    ] {
        for (kind, sql) in [
            ("first page", messages.explain(&query)),
            (
                "cursor page",
                messages.explain(&query.clone().after(ListCursor {
                    received_at: at(0),
                    id: MessageId::new(1),
                })),
            ),
        ] {
            let plan = plan_of(&connection, &sql);
            assert!(
                !plan.contains("TEMP B-TREE"),
                "{label} / {kind}: the list must never sort at query time:\n{plan}"
            );
            assert!(
                !plan.contains("SCAN messages"),
                "{label} / {kind}: the list must never scan the table:\n{plan}"
            );
            assert!(
                plan.contains("USING INDEX") || plan.contains("USING COVERING INDEX"),
                "{label} / {kind}: expected an index, got:\n{plan}"
            );
        }
    }
}

#[test]
fn paging_stays_flat_over_a_hundred_thousand_messages() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection);
    seed(&connection, inbox, 100_000);
    let messages = MessageRepository::new(&connection);

    let query = ListQuery::mailbox(inbox).limit(50);
    let time = |query: &ListQuery| -> (Duration, Vec<_>) {
        let start = Instant::now();
        let page = messages.page(query).expect("page");
        (start.elapsed(), page)
    };

    let (first_duration, first) = time(&query);
    assert_eq!(first.len(), 50, "a page is a window, never the mailbox");

    // Walk to the far end of the mailbox and time a page there.
    let mut cursor = first.last().expect("a row").cursor();
    let mut pages = 1;
    while pages < 1_900 {
        let page = messages.page(&query.clone().after(cursor)).expect("page");
        let Some(last) = page.last() else { break };
        cursor = last.cursor();
        pages += 1;
    }

    let (deep_duration, deep) = time(&query.clone().after(cursor));
    assert_eq!(deep.len(), 50, "still a full window, 95000 rows in");
    assert!(
        deep_duration < first_duration * 5 + Duration::from_millis(3),
        "keyset paging is a seek, not a skip: first {first_duration:?}, deep {deep_duration:?}"
    );

    // And the whole mailbox is never materialized: the only way to see every
    // row is to ask for one window at a time.
    assert_eq!(
        messages.count(&ListQuery::mailbox(inbox)).expect("count"),
        100_000
    );
}

// ---------------------------------------------------------------------------
// Reading an explicit, ranked set of ids
// ---------------------------------------------------------------------------

#[test]
fn rows_for_answers_in_the_order_it_was_asked() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    // Created oldest first, so id order and received_at order agree. That is
    // what makes "came back in the order asked" a claim about the argument
    // rather than a coincidence of how SQLite walked the table.
    let mut ids = Vec::new();
    for step in 0..5 {
        let mut message = a_message(inbox, account.id, step * 10);
        messages.create(&mut message).expect("create");
        ids.push(message.id);
    }

    // A ranking is neither of those orders. This one is deliberately not
    // sorted, not reverse-sorted, and not contiguous.
    let ranked = vec![ids[3], ids[0], ids[4], ids[1]];
    let rows = messages.rows_for(&ranked).expect("rows");

    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        ranked,
        "the rows came back in the store's order rather than the ranking"
    );
    // Real rows, not stubs: the list draws these.
    assert_eq!(rows[0].subject.as_deref(), Some("Re: Subject 30"));
    assert_eq!(
        rows[0].from.as_ref().map(|from| from.address.as_str()),
        Some("ada@example.com")
    );
    assert!(rows[0].seen, "the flags did not come with the row");
}

#[test]
fn rows_for_drops_what_the_store_no_longer_holds() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut ids = Vec::new();
    for step in 0..3 {
        let mut message = a_message(inbox, account.id, step * 10);
        messages.create(&mut message).expect("create");
        ids.push(message.id);
    }

    // The index and the store are allowed to disagree for a moment: a search
    // can hand back a message deleted between the query and this read. That
    // is a shorter answer, not an error, and certainly not a fabricated row.
    messages.delete(&[ids[1]]).expect("delete");

    let rows = messages.rows_for(&ids).expect("rows");
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![ids[0], ids[2]],
        "a deleted hit was faked or the survivors were reordered"
    );

    // Nothing asked for, nothing read -- and no SQL with an empty `IN ()`,
    // which SQLite rejects outright.
    assert!(messages.rows_for(&[]).expect("rows").is_empty());

    // An id that was never real is the same case.
    assert!(
        messages
            .rows_for(&[MessageId::new(999_999)])
            .expect("rows")
            .is_empty()
    );
}

// ── A resync must not resurrect what an undrained operation has moved ─────
//
// Archiving is local-first: the row moves to Archive in SQLite, a Move is
// queued, the list repaints. The server is told later, when the queue drains.
// In that window the server still lists the message in INBOX, so an INBOX
// resync fetches it and `upsert_batch` — which keys on (mailbox, validity,
// uid) — finds no row there and inserts a fresh one. The message the user
// just archived is back in the inbox, and stays there until the queue drains,
// which on a link that is down is indefinite (#368).

/// Enqueues `operation` against `message`, exactly as the local write does:
/// the enqueue snapshots the server coordinates before the local half nulls
/// them (#289).
fn enqueue_and_move_locally(
    connection: &Connection,
    account: postio_model::AccountId,
    message: MessageId,
    operation: &postio_model::Operation,
    destination: MailboxId,
) {
    use postio_storage::repository::OperationQueueRepository;
    OperationQueueRepository::new(connection)
        .enqueue(
            account,
            postio_model::OperationTarget::Message(message),
            operation,
            at(0),
        )
        .expect("enqueue");
    // The local half: the row moves, and its server coordinates go with the
    // queue row rather than staying on a message that is no longer there.
    connection
        .execute(
            "UPDATE messages SET mailbox_id = ?2, uid = NULL, uid_validity = NULL WHERE id = ?1",
            [message.get(), destination.get()],
        )
        .expect("local move");
}

fn rows_in(connection: &Connection, mailbox: MailboxId) -> usize {
    connection
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE mailbox_id = ?1",
            [mailbox.get()],
            |row| row.get::<_, i64>(0),
        )
        .expect("count") as usize
}

#[test]
fn a_resync_does_not_resurrect_a_message_with_an_undrained_move() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive");
    let messages = MessageRepository::new(&connection);

    // A message the server has in INBOX, synced normally.
    let mut batch = vec![a_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).expect("first sync");
    let message = batch[0].id;
    let (uid, validity) = (
        batch[0].server.uid.expect("uid"),
        batch[0].server.uid_validity.expect("validity"),
    );

    // The user archives it. Nothing has reached the server yet.
    enqueue_and_move_locally(
        &connection,
        account.id,
        message,
        &postio_model::Operation::Move {
            from: inbox,
            to: archive.id,
        },
        archive.id,
    );
    assert_eq!(
        rows_in(&connection, inbox),
        0,
        "the archive was local-first"
    );
    assert_eq!(rows_in(&connection, archive.id), 1);

    // Now an INBOX resync runs before the queue drains. The server still
    // lists the message in INBOX, so this is exactly what it hands back.
    let mut resynced = vec![a_message(inbox, account.id, 40)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    let report = messages.upsert_batch(&mut resynced).expect("resync upsert");

    assert_eq!(
        rows_in(&connection, inbox),
        0,
        "the archived message came back to the inbox: the resync re-created a \
         row the user had already moved, and it will sit there until the \
         queue drains (#368)"
    );
    assert_eq!(
        rows_in(&connection, archive.id),
        1,
        "and it must still be the one copy, in Archive where the user put it"
    );
    assert_eq!(
        report.shadowed_by_pending, 1,
        "the skip should be reported rather than silent"
    );
}

#[test]
fn a_resync_does_not_resurrect_a_message_with_an_undrained_delete() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let trash = test_support::mailbox(&connection, &account, "Trash");
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![a_message(inbox, account.id, 41)];
    messages.upsert_batch(&mut batch).expect("first sync");
    let message = batch[0].id;
    let (uid, validity) = (
        batch[0].server.uid.expect("uid"),
        batch[0].server.uid_validity.expect("validity"),
    );

    enqueue_and_move_locally(
        &connection,
        account.id,
        message,
        &postio_model::Operation::Delete {
            from: inbox,
            trash: trash.id,
        },
        trash.id,
    );

    let mut resynced = vec![a_message(inbox, account.id, 41)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    messages.upsert_batch(&mut resynced).expect("resync upsert");

    assert_eq!(
        rows_in(&connection, inbox),
        0,
        "a pending delete has the same shape as a pending move and needs the \
         same shadow (#368)"
    );
    assert_eq!(rows_in(&connection, trash.id), 1);
}

#[test]
fn the_shadow_lifts_once_the_operation_settles() {
    use postio_model::OperationState;
    use postio_storage::repository::OperationQueueRepository;

    for settled in [OperationState::Done, OperationState::Failed] {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let archive = test_support::mailbox(&connection, &account, "Archive");
        let messages = MessageRepository::new(&connection);

        let mut batch = vec![a_message(inbox, account.id, 42)];
        messages.upsert_batch(&mut batch).expect("first sync");
        let message = batch[0].id;
        let (uid, validity) = (
            batch[0].server.uid.expect("uid"),
            batch[0].server.uid_validity.expect("validity"),
        );

        enqueue_and_move_locally(
            &connection,
            account.id,
            message,
            &postio_model::Operation::Move {
                from: inbox,
                to: archive.id,
            },
            archive.id,
        );

        // The queue row settles, one way or the other.
        let queue = OperationQueueRepository::new(&connection);
        let pending = queue.pending(account.id, at(0)).expect("pending");
        let id = pending.first().expect("one queued row").id;
        match settled {
            OperationState::Done => queue.mark_done(id, at(1)).expect("done"),
            _ => queue
                .mark_failed(id, at(1), "server said no")
                .expect("failed"),
        }

        // A server that still lists the message in INBOX is now telling us
        // something we have to believe: either the move never happened, or
        // it happened and this is a genuinely different message at that UID.
        // Either way the shadow must be gone, or a failed move hides a
        // message for ever.
        let mut resynced = vec![a_message(inbox, account.id, 42)];
        resynced[0].server.uid = Some(uid);
        resynced[0].server.uid_validity = Some(validity);
        messages.upsert_batch(&mut resynced).expect("resync upsert");

        assert_eq!(
            rows_in(&connection, inbox),
            1,
            "{settled:?}: the shadow must lift when the operation settles, or \
             a move the server refused would hide the message for ever"
        );
    }
}

#[test]
fn the_sections_holding_a_message_s_text_round_trip() {
    // What the text axis fetches instead of `BODY.PEEK[]` (ADR 0017). The
    // header sync already parses these out of `BODYSTRUCTURE` and then throws
    // them away; without them the backfill cannot name the parts it wants and
    // has to pull the whole message, attachments included.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 202);
    message.text_part_id = Some("1.1".to_owned());
    message.html_part_id = Some("1.2".to_owned());
    let id = messages.create(&mut message).expect("create");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.text_part_id.as_deref(), Some("1.1"));
    assert_eq!(stored.html_part_id.as_deref(), Some("1.2"));
}

#[test]
fn a_message_synced_before_the_text_sections_existed_reads_back_as_none() {
    // The migration cannot invent these for rows already on disk, and
    // guessing `1` would be a wrong answer for every multipart message. NULL
    // is the honest "not known", and the backfill falls back to fetching the
    // whole message for such a row -- the same convention `content_type`
    // (migration 0004) set for this table.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 203);
    assert_eq!(message.text_part_id, None);
    assert_eq!(message.html_part_id, None);
    let id = messages.create(&mut message).expect("create");

    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.text_part_id, None);
    assert_eq!(stored.html_part_id, None);
}
