//! The message repository, and the windowed paging query the list depends on.
//!
//! Written before the repository existed. The bead's acceptance criteria are
//! "paging over a 100k-message fixture stays flat in time and memory", "the
//! sort key is stable across inserts (no row skipping while paging)" and a
//! recorded benchmark; the first two have tests here, and
//! `the_message_list_plan_never_sorts` is the structural half of the first.

use postio_storage::sql::bind;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    Attachment, BodyState, Disposition, EmailAddress, Flag, FlagSet, MailboxId, Message, MessageId,
    ModSeq, RfcMessageId, ThreadId, Uid, UidValidity,
};
use postio_storage::repository::{
    FlagSource, ListCursor, ListQuery, MessageRepository, StoredBody,
};
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
    message.server.remote_id = Some(postio_model::RemoteId::new(format!(
        "99:{}",
        seconds as u32 + 1
    )));
    message.server.mod_seq = Some(ModSeq::new(12_345));
    message.sync.body_state = BodyState::HeadersOnly;
    message
}

// ---------------------------------------------------------------------------
// Create, read, update, delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_message_round_trips_with_its_recipients_and_attachments() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
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

    let id = messages.create(&mut message).await.expect("create");

    assert!(id.is_assigned());
    for attachment in &message.attachments {
        assert!(attachment.id.is_assigned(), "attachments get ids too");
        assert_eq!(attachment.message_id, id);
    }

    let stored = messages.get(id).await.expect("get").expect("the message");
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

#[tokio::test]
async fn a_message_s_own_content_type_round_trips() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 200);
    message.content_type = Some("multipart/related".to_owned());
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.content_type.as_deref(), Some("multipart/related"));
}

#[tokio::test]
async fn a_message_with_no_content_type_recorded_reads_back_as_none() {
    // A row synced before this field existed, or a draft nothing has parsed
    // `BODYSTRUCTURE` for yet -- distinct from an empty string, which would
    // be a wrong answer rather than an honest "not known".
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 201);
    assert_eq!(message.content_type, None);
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.content_type, None);
}

#[tokio::test]
async fn a_message_s_text_is_flowed_flag_round_trips() {
    // #456: a reply built from a message loaded back out of storage needs
    // this to survive the round trip, or every message a user actually
    // replies to (loaded from the database, never straight off the parser)
    // would silently lose it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 202);
    message.text_is_flowed = true;
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert!(stored.text_is_flowed);
}

#[tokio::test]
async fn a_message_with_no_flag_recorded_reads_back_as_not_flowed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 203);
    assert!(!message.text_is_flowed);
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert!(!stored.text_is_flowed);
}

#[tokio::test]
async fn a_message_s_list_id_round_trips() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 300);
    message.list_id = Some("harbour-dev.lists.example.org".to_owned());
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(
        stored.list_id.as_deref(),
        Some("harbour-dev.lists.example.org")
    );
}

#[tokio::test]
async fn a_message_with_no_list_id_reads_back_as_none() {
    // Most mail is not list mail; a `None` here must stay `None`, not an
    // empty string that would misread as a list with no name.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 301);
    assert_eq!(message.list_id, None);
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.list_id, None);
}

#[tokio::test]
async fn flags_are_denormalized_so_the_list_never_parses_a_string() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 1);
    message.flags = [Flag::Seen, Flag::Flagged, Flag::Answered, Flag::Recent]
        .into_iter()
        .collect();
    let id = messages.create(&mut message).await.expect("create");

    let (flags, seen, flagged, answered, draft): (String, bool, bool, bool, bool) =
        postio_storage::sql::one(
            &connection,
            "SELECT flags, seen, flagged, answered, draft FROM messages WHERE id = ?1",
            bind![id.get()],
            |row| {
                Ok((
                    postio_storage::sql::RowExt::col(row, 0)?,
                    postio_storage::sql::RowExt::col(row, 1)?,
                    postio_storage::sql::RowExt::col(row, 2)?,
                    postio_storage::sql::RowExt::col(row, 3)?,
                    postio_storage::sql::RowExt::col(row, 4)?,
                ))
            },
        )
        .await
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
            .await
            .expect("get")
            .expect("the message")
            .flags
            .contains(&Flag::Recent)
    );
}

#[tokio::test]
async fn updating_a_message_replaces_its_recipients_rather_than_appending() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 2);
    let id = messages.create(&mut message).await.expect("create");

    message.to = vec![EmailAddress::new(None::<String>, "only@example.com")];
    message.subject = Some("Rewritten".to_owned());
    message.attachments.clear();
    messages.update(&mut message).await.expect("update");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.to.len(), 1);
    assert_eq!(stored.subject.as_deref(), Some("Rewritten"));

    let recipients: i64 =
        postio_storage::sql::one(&connection, "SELECT count(*) FROM recipients", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("count");
    assert_eq!(recipients, 3, "from + to + cc, with no leftovers");
}

#[tokio::test]
async fn deleting_messages_takes_their_recipients_and_attachments() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut first = a_message(inbox, account.id, 3);
    let mut second = a_message(inbox, account.id, 4);
    let first_id = messages.create(&mut first).await.expect("create");
    let second_id = messages.create(&mut second).await.expect("create");

    assert_eq!(
        messages
            .delete(&[first_id, second_id])
            .await
            .expect("delete"),
        2
    );
    assert!(messages.get(first_id).await.expect("get").is_none());

    for table in ["messages", "recipients", "attachments"] {
        let remaining: i64 = postio_storage::sql::one(
            &connection,
            &format!("SELECT count(*) FROM {table}"),
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("count");
        assert_eq!(remaining, 0, "{table}");
    }
    assert_eq!(
        messages.delete(&[first_id]).await.expect("delete again"),
        0,
        "deleting what is gone is zero, not an error"
    );
}

/// The body is stored on the row (ADR 0020) but is *not* carried on the
/// [`Message`] a `get` hands back.
///
/// That is the whole reason `body` is a separate call: the list pages over
/// rows of a few hundred bytes, and a body on every `Message` would put the
/// mailbox in memory. `tests/body.rs` holds down the round trip itself.
#[tokio::test]
async fn a_read_message_does_not_carry_its_body_and_the_raw_blob_key_survives() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 5);
    // The raw `.eml` is still a blob: large, streamed, deduplicated.
    message.raw_blob_id = Some(postio_model::BlobId::new("a".repeat(64)));
    let id = messages.create(&mut message).await.expect("create");

    assert_eq!(
        messages.body(id).await.expect("body").expect("the row"),
        StoredBody::default(),
        "nothing has been downloaded yet"
    );

    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("the plain text".to_owned()),
                html: Some("<p>the html</p>".to_owned()),
                headers: Some("Subject: hello\r\n".to_owned()),
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("set");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.raw_blob_id, message.raw_blob_id);
    assert_eq!(stored.sync.body_state, BodyState::Full);
    assert!(
        stored.body.is_empty() && stored.headers.is_empty(),
        "a listed message does not drag its body along; `body` is the way to it"
    );
}

// ---------------------------------------------------------------------------
// Backfill candidates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn needing_backfill_returns_newest_first_and_skips_full_bodies() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut oldest = a_message(inbox, account.id, 1);
    let mut newest = a_message(inbox, account.id, 3);
    let mut already_full = a_message(inbox, account.id, 2);
    already_full.sync.body_state = BodyState::Full;

    for message in [&mut oldest, &mut newest, &mut already_full] {
        messages.create(message).await.expect("create");
    }

    let candidates = messages.needing_backfill(inbox, 10).await.expect("query");
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

#[tokio::test]
async fn needing_backfill_is_windowed_and_scoped_to_its_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let messages = MessageRepository::new(&connection);

    for seconds in 0..5 {
        messages
            .create(&mut a_message(inbox, account.id, seconds))
            .await
            .expect("create");
    }
    messages
        .create(&mut a_message(archive.id, account.id, 99))
        .await
        .expect("create in another mailbox");

    let limited = messages.needing_backfill(inbox, 2).await.expect("query");
    assert_eq!(limited.len(), 2, "the window caps how many come back");

    let archived = messages
        .needing_backfill(archive.id, 10)
        .await
        .expect("query");
    assert_eq!(
        archived.len(),
        1,
        "a mailbox never sees another mailbox's candidates"
    );
}

#[tokio::test]
async fn a_message_with_no_uid_yet_is_not_a_backfill_candidate() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut composed = a_message(inbox, account.id, 1);
    composed.server.uid = None;
    messages.create(&mut composed).await.expect("create");

    assert!(
        messages
            .needing_backfill(inbox, 10)
            .await
            .expect("query")
            .is_empty()
    );
    assert!(
        messages
            .backfill_candidate(composed.id)
            .await
            .expect("query")
            .is_none()
    );
}

#[tokio::test]
async fn backfill_candidate_looks_up_a_single_message_by_id_alone() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 7);
    messages.create(&mut message).await.expect("create");

    let candidate = messages
        .backfill_candidate(message.id)
        .await
        .expect("query")
        .expect("a candidate, since the body is headers-only");
    assert_eq!(candidate.mailbox_id, inbox);
    assert_eq!(candidate.mailbox_path, "INBOX");
    assert_eq!(candidate.uid, message.server.uid.unwrap());

    messages
        .set_body(message.id, &StoredBody::default(), BodyState::Full)
        .await
        .expect("mark it fetched");
    assert!(
        messages
            .backfill_candidate(message.id)
            .await
            .expect("query")
            .is_none(),
        "a message that already has its full body is not a candidate"
    );
}

// ---------------------------------------------------------------------------
// Batch upsert, the shape sync writes in
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upserting_a_batch_inserts_what_is_new_and_updates_what_is_known() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![
        a_message(inbox, account.id, 10),
        a_message(inbox, account.id, 11),
    ];
    let report = messages
        .upsert_batch(&mut batch)
        .await
        .expect("first upsert");
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
    let report = messages
        .upsert_batch(&mut again)
        .await
        .expect("second upsert");

    assert_eq!(report.inserted, 1);
    assert_eq!(report.updated, 2);
    assert_eq!(
        again[0].id, ids[0],
        "a message keeps its local id across a resync, so the UI's selection survives"
    );

    let total: i64 =
        postio_storage::sql::one(&connection, "SELECT count(*) FROM messages", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("count");
    assert_eq!(total, 3, "no duplicates");
    assert!(
        messages
            .get(ids[0])
            .await
            .expect("get")
            .expect("the message")
            .flags
            .is_flagged()
    );
}

#[tokio::test]
async fn a_locally_composed_message_with_no_uid_is_never_matched_by_upsert() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut first = a_message(inbox, account.id, 20);
    first.server = Default::default();
    let mut second = a_message(inbox, account.id, 21);
    second.server = Default::default();

    let mut batch = vec![first, second];
    let report = messages.upsert_batch(&mut batch).await.expect("upsert");

    assert_eq!(
        report.inserted, 2,
        "with no server identity there is nothing to match on"
    );
    assert_ne!(batch[0].id, batch[1].id);
}

// ---------------------------------------------------------------------------
// Lookups
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_message_can_be_found_by_its_server_uid() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 30);
    let id = messages.create(&mut message).await.expect("create");
    let uid = message.server.uid.unwrap();
    let validity = postio_model::Generation::new(message.server.uid_validity.unwrap().get());

    assert_eq!(
        messages
            .by_uid(inbox, validity, uid)
            .await
            .expect("by uid")
            .map(|message| message.id),
        Some(id)
    );
    assert!(
        messages
            .by_uid(inbox, postio_model::Generation::new(100), uid)
            .await
            .expect("by uid")
            .is_none(),
        "a UID means nothing under a different UIDVALIDITY"
    );
    assert_eq!(
        messages.uids_in(inbox, validity).await.expect("uids"),
        vec![uid]
    );
    assert!(
        messages
            .uids_in(inbox, postio_model::Generation::new(100))
            .await
            .expect("uids")
            .is_empty()
    );
}

#[tokio::test]
async fn messages_can_be_found_by_rfc_message_id_and_duplicates_all_come_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let shared = RfcMessageId::new("<shared@example.com>");
    let mut first = a_message(inbox, account.id, 40);
    first.rfc_message_id = Some(shared.clone());
    let mut second = a_message(inbox, account.id, 41);
    second.rfc_message_id = Some(shared.clone());
    let first_id = messages.create(&mut first).await.expect("create");
    let second_id = messages.create(&mut second).await.expect("create");

    let found = messages
        .ids_by_rfc_message_id(account.id, &shared)
        .await
        .expect("lookup");

    assert_eq!(
        found,
        vec![first_id, second_id],
        "a Message-ID is not unique in the wild; the corpus has a fixture that reuses one"
    );
    assert!(
        messages
            .ids_by_rfc_message_id(account.id, &RfcMessageId::new("nothing@example.com"))
            .await
            .expect("lookup")
            .is_empty()
    );
    assert_eq!(
        messages
            .ids_by_rfc_message_id(account.id, &RfcMessageId::new("<SHARED@EXAMPLE.COM>"))
            .await
            .expect("lookup"),
        vec![first_id, second_id],
        "Message-IDs compare case-insensitively, the way threading needs"
    );
}

// ---------------------------------------------------------------------------
// Flags, moves, local delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_local_flag_change_marks_the_row_dirty_and_a_server_one_does_not() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 50);
    let id = messages.create(&mut message).await.expect("create");

    let mut flags = FlagSet::new();
    flags.insert(Flag::Flagged);
    messages
        .set_flags(id, &flags, FlagSource::Local)
        .await
        .expect("local change");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert!(stored.flags.is_flagged() && !stored.flags.is_seen());
    assert!(
        stored.sync.flags_dirty,
        "a local change is ahead of the server until it is pushed"
    );

    messages
        .set_flags(id, &flags, FlagSource::Server)
        .await
        .expect("server change");
    assert!(
        !messages
            .get(id)
            .await
            .expect("get")
            .expect("the message")
            .sync
            .flags_dirty,
        "what the server told us is by definition not ahead of it"
    );
}

#[tokio::test]
async fn moving_a_message_drops_the_uid_it_had_in_the_old_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 60);
    let id = messages.create(&mut message).await.expect("create");

    assert_eq!(messages.move_to(&[id], archive.id).await.expect("move"), 1);

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.mailbox_id, archive.id);
    assert_eq!(
        stored.server.uid, None,
        "a UID belongs to the mailbox it was issued in"
    );
    assert_eq!(stored.server.uid_validity, None);
    assert_eq!(
        stored.server.remote_id, None,
        "`remote_id` is \"<uid_validity>:<uid>\" -- the same coordinate as the \
         two fields above, spelled as one string. Left behind, it reads as the \
         row's identity in a mailbox it was never issued in, and the next \
         command on that message compares the source folder's generation \
         against the destination's and reports a UIDVALIDITY change that never \
         happened."
    );
    assert!(
        stored.sync.has_pending_operations,
        "the move still has to be pushed"
    );
}

#[tokio::test]
async fn a_locally_deleted_message_is_hidden_from_the_list_but_still_there() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 70);
    let id = messages.create(&mut message).await.expect("create");

    assert_eq!(
        messages
            .set_deleted_locally(&[id], true)
            .await
            .expect("hide"),
        1
    );
    assert!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .await
            .expect("page")
            .is_empty(),
        "the list hides it the instant the user presses the key"
    );
    assert!(
        messages.get(id).await.expect("get").is_some(),
        "but undo has to be able to bring it back"
    );

    messages
        .set_deleted_locally(&[id], false)
        .await
        .expect("undo");
    assert_eq!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .await
            .expect("page")
            .len(),
        1
    );
}

#[tokio::test]
async fn a_snoozed_message_leaves_every_ordinary_scope_and_appears_in_snoozed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 70);
    let id = messages.create(&mut message).await.expect("create");
    let until = Utc::now() + Duration::from_secs(3600);

    assert_eq!(messages.snooze(&[id], until).await.expect("snooze"), 1);

    assert!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .await
            .expect("page")
            .is_empty(),
        "a snoozed message must leave the folder it is filed in"
    );
    assert!(
        messages
            .page(&ListQuery::account(account.id))
            .await
            .expect("page")
            .is_empty(),
        "and the unified account view too, or it would just reappear there"
    );
    assert_eq!(
        messages
            .page(&ListQuery::snoozed(account.id))
            .await
            .expect("page")
            .len(),
        1,
        "but the whole point is that it is still findable, in its own view"
    );
    assert!(
        messages.get(id).await.expect("get").is_some(),
        "snoozing is not deleting"
    );
}

#[tokio::test]
async fn waking_due_snoozes_clears_only_what_is_due_and_says_which_mailboxes_changed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let messages = MessageRepository::new(&connection);

    let mut due = a_message(inbox, account.id, 70);
    let due_id = messages.create(&mut due).await.expect("create due");
    let mut not_due = a_message(archive.id, account.id, 71);
    let not_due_id = messages.create(&mut not_due).await.expect("create not due");

    let now = Utc::now();
    messages
        .snooze(&[due_id], now - Duration::from_secs(1))
        .await
        .expect("snooze the one whose time has already come");
    messages
        .snooze(&[not_due_id], now + Duration::from_secs(3600))
        .await
        .expect("snooze the one whose time has not come");

    let woken = messages.wake_due(account.id, now).await.expect("wake due");
    assert_eq!(
        woken,
        vec![inbox],
        "only the folder holding the due message should be told to repaint"
    );

    assert_eq!(
        messages
            .get(due_id)
            .await
            .expect("get")
            .unwrap()
            .snoozed_until,
        None,
        "waking clears the snooze rather than merely revealing it"
    );
    assert!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .await
            .expect("page")
            .iter()
            .any(|row| row.id == due_id),
        "the due message is back in its folder"
    );
    assert!(
        messages
            .page(&ListQuery::mailbox(archive.id))
            .await
            .expect("page")
            .is_empty(),
        "the one whose time has not come stays hidden"
    );

    assert_eq!(
        messages
            .wake_due(account.id, now)
            .await
            .expect("wake due again"),
        Vec::new(),
        "nothing left to wake, so nothing left to repaint"
    );
}

#[tokio::test]
async fn waking_due_snoozes_never_touches_another_accounts_rows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account_a, inbox_a) = test_support::account_with_inbox(&connection).await;
    let (account_b, inbox_b) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message_a = a_message(inbox_a, account_a.id, 70);
    let id_a = messages.create(&mut message_a).await.expect("create a");
    let mut message_b = a_message(inbox_b, account_b.id, 71);
    let id_b = messages.create(&mut message_b).await.expect("create b");

    let now = Utc::now();
    // Truncated to millisecond precision: that is what the column stores,
    // and the round trip below has to match it exactly.
    let due =
        DateTime::from_timestamp_millis((now - Duration::from_secs(1)).timestamp_millis()).unwrap();
    messages.snooze(&[id_a], due).await.expect("snooze a");
    messages.snooze(&[id_b], due).await.expect("snooze b");

    assert_eq!(
        messages.wake_due(account_a.id, now).await.expect("wake a"),
        vec![inbox_a],
        "only account a's engine asked, so only account a's row may wake"
    );
    assert_eq!(
        messages
            .get(id_b)
            .await
            .expect("get b")
            .unwrap()
            .snoozed_until,
        Some(due),
        "account b's own engine has not ticked yet, so its snooze must stand"
    );
}

#[tokio::test]
async fn unsnoozing_clears_it_immediately_without_waiting_for_wake_due() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 70);
    let id = messages.create(&mut message).await.expect("create");
    messages
        .snooze(&[id], Utc::now() + Duration::from_secs(3600))
        .await
        .expect("snooze");

    assert_eq!(messages.unsnooze(&[id]).await.expect("unsnooze"), 1);
    assert_eq!(
        messages.get(id).await.expect("get").unwrap().snoozed_until,
        None
    );
    assert_eq!(
        messages
            .page(&ListQuery::mailbox(inbox))
            .await
            .expect("page")
            .len(),
        1,
        "cancelling a snooze has to be as immediate as pressing the key"
    );
}

// ---------------------------------------------------------------------------
// The windowed list query
// ---------------------------------------------------------------------------

/// Inserts `count` messages straight into the table, newest last.
///
/// Raw SQL and one statement: this is the fixture for the paging tests, and
/// building it through the repository would make them a test of insert speed.
async fn seed(connection: &Connection, mailbox: MailboxId, count: u32) {
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
            bind![mailbox.get(), count],
        )
        .await
        .expect("seed messages");
    connection
        .execute(
            "INSERT INTO addresses (address, address_normalized)
             SELECT 'sender' || id || '@example.com', 'sender' || id || '@example.com'
               FROM messages WHERE mailbox_id = ?1
             ON CONFLICT (address_normalized) DO NOTHING",
            [mailbox.get()],
        )
        .await
        .expect("seed addresses");
    connection
        .execute(
            "INSERT INTO recipients (message_id, kind, position, name, address_id)
             SELECT m.id, 'from', 0, 'Sender ' || m.id, a.id
               FROM messages m
               JOIN addresses a
                 ON a.address_normalized = 'sender' || m.id || '@example.com'
              WHERE m.mailbox_id = ?1",
            [mailbox.get()],
        )
        .await
        .expect("seed senders");
}

#[tokio::test]
async fn a_page_is_newest_first_and_no_longer_than_the_window() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection).await;
    seed(&connection, inbox, 10).await;
    let messages = MessageRepository::new(&connection);

    let page = messages
        .page(&ListQuery::mailbox(inbox).limit(4))
        .await
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

#[tokio::test]
async fn paging_with_a_cursor_walks_the_whole_mailbox_exactly_once() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection).await;
    seed(&connection, inbox, 250).await;
    let messages = MessageRepository::new(&connection);

    let mut seen: Vec<MessageId> = Vec::new();
    let mut cursor: Option<ListCursor> = None;
    loop {
        let mut query = ListQuery::mailbox(inbox).limit(40);
        if let Some(cursor) = cursor {
            query = query.after(cursor);
        }
        let page = messages.page(&query).await.expect("page");
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

#[tokio::test]
async fn a_message_arriving_mid_scroll_does_not_make_the_list_skip_a_row() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    seed(&connection, inbox, 100).await;
    let messages = MessageRepository::new(&connection);

    let first = messages
        .page(&ListQuery::mailbox(inbox).limit(10))
        .await
        .expect("first page");
    let cursor = first.last().expect("a row").cursor();

    // IDLE delivers a new message at the top while the user is still scrolling.
    let mut arrival = a_message(inbox, account.id, 1_000_000);
    messages.create(&mut arrival).await.expect("create");

    let second = messages
        .page(&ListQuery::mailbox(inbox).limit(10).after(cursor))
        .await
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

#[tokio::test]
async fn paging_by_offset_is_available_for_a_windowed_list_model() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection).await;
    seed(&connection, inbox, 100).await;
    let messages = MessageRepository::new(&connection);

    let page = messages
        .page_at(&ListQuery::mailbox(inbox).limit(5), 20)
        .await
        .expect("page at an offset");

    assert_eq!(page.len(), 5);
    assert_eq!(
        page[0].subject.as_deref(),
        Some("Subject 80"),
        "row 21 counting from the newest"
    );
    assert_eq!(
        messages
            .count(&ListQuery::mailbox(inbox))
            .await
            .expect("count"),
        100
    );
}

#[tokio::test]
async fn the_list_can_be_scoped_to_an_account_or_to_flagged_messages() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    seed(&connection, inbox, 10).await;
    seed(&connection, archive.id, 10).await;
    let messages = MessageRepository::new(&connection);

    assert_eq!(
        messages
            .count(&ListQuery::mailbox(inbox))
            .await
            .expect("count"),
        10
    );
    assert_eq!(
        messages
            .count(&ListQuery::account(account.id))
            .await
            .expect("count"),
        20,
        "the unified view spans mailboxes"
    );
    assert_eq!(
        messages
            .count(&ListQuery::flagged(account.id))
            .await
            .expect("count"),
        2,
        "every seventh message, in each of the two mailboxes"
    );
    assert!(
        messages
            .page(&ListQuery::flagged(account.id))
            .await
            .expect("page")
            .iter()
            .all(|row| row.flagged)
    );
}

#[tokio::test]
async fn a_thread_id_travels_on_the_list_row_so_the_list_can_group_without_a_second_query() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 80);
    let id = messages.create(&mut message).await.expect("create");
    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1)",
            [account.id.get()],
        )
        .await
        .expect("a thread");
    messages
        .set_thread(id, Some(ThreadId::new(1)))
        .await
        .expect("assign");

    let page = messages
        .page(&ListQuery::mailbox(inbox))
        .await
        .expect("page");
    assert_eq!(page[0].thread_id, Some(ThreadId::new(1)));
    assert_eq!(
        messages
            .get(id)
            .await
            .expect("get")
            .expect("the message")
            .thread_id,
        Some(ThreadId::new(1))
    );
}

#[tokio::test]
async fn a_thread_is_a_scope_of_its_own_so_a_drill_in_is_not_limited_to_one_folder() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let messages = MessageRepository::new(&connection);
    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1), (2, ?1)",
            [account.id.get()],
        )
        .await
        .expect("two threads");

    // A conversation half of which has been archived, which is what an
    // ordinary thread looks like after anyone has tidied up — plus a second
    // thread in the same folder, so a scope that ignored the thread entirely
    // would be caught rather than passing by luck.
    let mut ours = Vec::new();
    for (mailbox, minute) in [(inbox, 10), (archive.id, 20), (inbox, 30), (archive.id, 40)] {
        let mut message = a_message(mailbox, account.id, minute);
        let id = messages.create(&mut message).await.expect("create");
        messages
            .set_thread(id, Some(ThreadId::new(1)))
            .await
            .expect("assign");
        ours.push(id);
    }
    let mut other = a_message(inbox, account.id, 50);
    let elsewhere = messages.create(&mut other).await.expect("create");
    messages
        .set_thread(elsewhere, Some(ThreadId::new(2)))
        .await
        .expect("assign");

    let thread = ListQuery::thread(ThreadId::new(1));
    assert_eq!(
        messages.count(&thread).await.expect("count"),
        4,
        "the thread spans two folders and the scope has to span them too"
    );

    let page = messages.page(&thread).await.expect("page");
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
async fn plan_of(connection: &Connection, sql: &str) -> String {
    test_support::plan(connection, sql).await
}

#[tokio::test]
async fn the_message_list_plan_never_sorts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection).await;
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
            let plan = plan_of(&connection, &sql).await;
            assert!(
                !postio_storage::test_support::sorts(&plan),
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

/// A cursor page seeks past the cursor rather than filtering down to it.
///
/// The clock version of this is `paging_stays_flat_over_a_hundred_thousand_messages`,
/// which seeds 100,000 rows and takes two minutes. This asks the same question
/// of the planner in a tenth of a second, and it is the one that will say
/// *why*: an index term naming the sort column means a seek, and its absence
/// means the engine found the scope, then walked every row above the cursor
/// testing each one. Measured before the fix, that was 1.4 ms for the first
/// page against 107 ms for a page 95,000 rows in.
///
/// The spelling that produces the term is in `where_clause`, and it is not the
/// obvious one -- a row value comparison, which SQLite turns into exactly this
/// range constraint, this engine plans as a filter. So the assertion is about
/// the plan and not about the SQL: what matters is that the cursor *reaches*
/// the index, however it is written.
#[tokio::test]
async fn a_cursor_page_seeks_past_the_cursor_instead_of_filtering_down_to_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);

    for (label, query, sort_column) in [
        (
            "mailbox",
            ListQuery::mailbox(MailboxId::new(1)),
            "received_at",
        ),
        (
            "account",
            ListQuery::account(postio_model::AccountId::new(1)),
            "received_at",
        ),
    ] {
        let sql = messages.explain(&query.clone().after(ListCursor {
            received_at: at(0),
            id: MessageId::new(1),
        }));
        let plan = plan_of(&connection, &sql).await;
        assert!(
            plan.contains(sort_column),
            "{label}: the cursor never reaches the index -- the engine seeks \
             the scope and then filters every row above the cursor, which is \
             the skip keyset paging exists to avoid:\n{plan}"
        );
    }
}

#[tokio::test]
async fn paging_stays_flat_over_a_hundred_thousand_messages() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = test_support::account_with_inbox(&connection).await;
    seed(&connection, inbox, 100_000).await;
    let messages = MessageRepository::new(&connection);

    let query = ListQuery::mailbox(inbox).limit(50);
    let time = async |query: &ListQuery| -> (Duration, Vec<_>) {
        let start = Instant::now();
        let page = messages.page(query).await.expect("page");
        (start.elapsed(), page)
    };

    let (first_duration, first) = time(&query).await;
    assert_eq!(first.len(), 50, "a page is a window, never the mailbox");

    // Walk to the far end of the mailbox and time a page there.
    let mut cursor = first.last().expect("a row").cursor();
    let mut pages = 1;
    while pages < 1_900 {
        let page = messages
            .page(&query.clone().after(cursor))
            .await
            .expect("page");
        let Some(last) = page.last() else { break };
        cursor = last.cursor();
        pages += 1;
    }

    let (deep_duration, deep) = time(&query.clone().after(cursor)).await;
    assert_eq!(deep.len(), 50, "still a full window, 95000 rows in");
    assert!(
        deep_duration < first_duration * 5 + Duration::from_millis(3),
        "keyset paging is a seek, not a skip: first {first_duration:?}, deep {deep_duration:?}"
    );

    // And the whole mailbox is never materialized: the only way to see every
    // row is to ask for one window at a time.
    assert_eq!(
        messages
            .count(&ListQuery::mailbox(inbox))
            .await
            .expect("count"),
        100_000
    );
}

// ---------------------------------------------------------------------------
// Reading an explicit, ranked set of ids
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rows_for_answers_in_the_order_it_was_asked() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    // Created oldest first, so id order and received_at order agree. That is
    // what makes "came back in the order asked" a claim about the argument
    // rather than a coincidence of how SQLite walked the table.
    let mut ids = Vec::new();
    for step in 0..5 {
        let mut message = a_message(inbox, account.id, step * 10);
        messages.create(&mut message).await.expect("create");
        ids.push(message.id);
    }

    // A ranking is neither of those orders. This one is deliberately not
    // sorted, not reverse-sorted, and not contiguous.
    let ranked = vec![ids[3], ids[0], ids[4], ids[1]];
    let rows = messages.rows_for(&ranked).await.expect("rows");

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

#[tokio::test]
async fn rows_for_drops_what_the_store_no_longer_holds() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut ids = Vec::new();
    for step in 0..3 {
        let mut message = a_message(inbox, account.id, step * 10);
        messages.create(&mut message).await.expect("create");
        ids.push(message.id);
    }

    // The index and the store are allowed to disagree for a moment: a search
    // can hand back a message deleted between the query and this read. That
    // is a shorter answer, not an error, and certainly not a fabricated row.
    messages.delete(&[ids[1]]).await.expect("delete");

    let rows = messages.rows_for(&ids).await.expect("rows");
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![ids[0], ids[2]],
        "a deleted hit was faked or the survivors were reordered"
    );

    // Nothing asked for, nothing read -- and no SQL with an empty `IN ()`,
    // which SQLite rejects outright.
    assert!(messages.rows_for(&[]).await.expect("rows").is_empty());

    // An id that was never real is the same case.
    assert!(
        messages
            .rows_for(&[MessageId::new(999_999)])
            .await
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
async fn enqueue_and_move_locally(
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
        .await
        .expect("enqueue");
    // The local half: the row moves, and its server coordinates go with the
    // queue row rather than staying on a message that is no longer there.
    connection
        .execute(
            "UPDATE messages SET mailbox_id = ?2, uid = NULL, uid_validity = NULL, remote_id = NULL
              WHERE id = ?1",
            [message.get(), destination.get()],
        )
        .await
        .expect("local move");
}

async fn rows_in(connection: &Connection, mailbox: MailboxId) -> usize {
    postio_storage::sql::one(
        connection,
        "SELECT COUNT(*) FROM messages WHERE mailbox_id = ?1",
        bind![mailbox.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
    )
    .await
    .expect("count") as usize
}

#[tokio::test]
async fn a_resync_does_not_resurrect_a_message_with_an_undrained_move() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let messages = MessageRepository::new(&connection);

    // A message the server has in INBOX, synced normally.
    let mut batch = vec![a_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
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
    )
    .await;
    assert_eq!(
        rows_in(&connection, inbox).await,
        0,
        "the archive was local-first"
    );
    assert_eq!(rows_in(&connection, archive.id).await, 1);

    // Now an INBOX resync runs before the queue drains. The server still
    // lists the message in INBOX, so this is exactly what it hands back.
    let mut resynced = vec![a_message(inbox, account.id, 40)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    resynced[0].server.remote_id = Some(postio_model::RemoteId::new(format!("{validity}:{uid}")));
    let report = messages
        .upsert_batch(&mut resynced)
        .await
        .expect("resync upsert");

    assert_eq!(
        rows_in(&connection, inbox).await,
        0,
        "the archived message came back to the inbox: the resync re-created a \
         row the user had already moved, and it will sit there until the \
         queue drains (#368)"
    );
    assert_eq!(
        rows_in(&connection, archive.id).await,
        1,
        "and it must still be the one copy, in Archive where the user put it"
    );
    assert_eq!(
        report.shadowed_by_pending, 1,
        "the skip should be reported rather than silent"
    );
}

#[tokio::test]
async fn a_resync_does_not_resurrect_a_message_with_an_undrained_delete() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let trash = test_support::mailbox(&connection, &account, "Trash").await;
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![a_message(inbox, account.id, 41)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
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
    )
    .await;

    let mut resynced = vec![a_message(inbox, account.id, 41)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    resynced[0].server.remote_id = Some(postio_model::RemoteId::new(format!("{validity}:{uid}")));
    messages
        .upsert_batch(&mut resynced)
        .await
        .expect("resync upsert");

    assert_eq!(
        rows_in(&connection, inbox).await,
        0,
        "a pending delete has the same shape as a pending move and needs the \
         same shadow (#368)"
    );
    assert_eq!(rows_in(&connection, trash.id).await, 1);
}

#[tokio::test]
async fn the_shadow_lifts_once_the_operation_settles() {
    use postio_model::OperationState;
    use postio_storage::repository::OperationQueueRepository;

    for settled in [OperationState::Done, OperationState::Failed] {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let archive = test_support::mailbox(&connection, &account, "Archive").await;
        let messages = MessageRepository::new(&connection);

        let mut batch = vec![a_message(inbox, account.id, 42)];
        messages.upsert_batch(&mut batch).await.expect("first sync");
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
        )
        .await;

        // The queue row settles, one way or the other.
        let queue = OperationQueueRepository::new(&connection);
        let pending = queue.pending(account.id, at(0)).await.expect("pending");
        let id = pending.first().expect("one queued row").id;
        match settled {
            OperationState::Done => queue.mark_done(id, at(1)).await.expect("done"),
            _ => queue
                .mark_failed(id, at(1), "server said no")
                .await
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
        messages
            .upsert_batch(&mut resynced)
            .await
            .expect("resync upsert");

        assert_eq!(
            rows_in(&connection, inbox).await,
            1,
            "{settled:?}: the shadow must lift when the operation settles, or \
             a move the server refused would hide the message for ever"
        );
    }
}

#[tokio::test]
async fn the_sections_holding_a_message_s_text_round_trip() {
    // What the text axis fetches instead of `BODY.PEEK[]` (ADR 0017). The
    // header sync already parses these out of `BODYSTRUCTURE` and then throws
    // them away; without them the backfill cannot name the parts it wants and
    // has to pull the whole message, attachments included.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 202);
    message.text_part_id = Some("1.1".to_owned());
    message.html_part_id = Some("1.2".to_owned());
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.text_part_id.as_deref(), Some("1.1"));
    assert_eq!(stored.html_part_id.as_deref(), Some("1.2"));
}

#[tokio::test]
async fn a_message_synced_before_the_text_sections_existed_reads_back_as_none() {
    // The migration cannot invent these for rows already on disk, and
    // guessing `1` would be a wrong answer for every multipart message. NULL
    // is the honest "not known", and the backfill falls back to fetching the
    // whole message for such a row -- the same convention `content_type`
    // (migration 0004) set for this table.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 203);
    assert_eq!(message.text_part_id, None);
    assert_eq!(message.html_part_id, None);
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.text_part_id, None);
    assert_eq!(stored.html_part_id, None);
}

#[tokio::test]
async fn a_message_whose_text_is_local_is_not_queued_for_backfill_again() {
    // The bug the e2e gate caught. `partial` means text local, payloads not
    // (ADR 0017), and it is a *settled* state -- there is nothing more the
    // background lane should do for such a message.
    //
    // While this query asked for `body_state <> 'full'`, every text-backfilled
    // message carrying an attachment came straight back as a candidate: fetch
    // its text, store it, settle at `partial`, and be handed back by the very
    // next seed. The backfill spun on one message forever and starved
    // everything behind it, including newly arriving mail.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut settled = a_message(inbox, account.id, 300);
    settled.sync.body_state = BodyState::Partial;
    messages.create(&mut settled).await.expect("create");

    let mut wanted = a_message(inbox, account.id, 301);
    wanted.sync.body_state = BodyState::HeadersOnly;
    let wanted_id = messages.create(&mut wanted).await.expect("create");

    let candidates = messages
        .needing_backfill_from(inbox, 10, 0)
        .await
        .expect("candidates");

    assert_eq!(
        candidates.iter().map(|c| c.message_id).collect::<Vec<_>>(),
        vec![wanted_id],
        "only the message with no text yet"
    );
}

#[tokio::test]
async fn a_partial_message_is_still_reachable_by_the_interactive_lane() {
    // The other side of it. `partial` is settled for the *background* lane,
    // not for the user: opening an attachment on such a message has to be
    // able to ask for the parts the background lane deliberately declined,
    // so the interactive lookup still answers for it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 302);
    message.sync.body_state = BodyState::Partial;
    let id = messages.create(&mut message).await.expect("create");

    assert!(
        messages
            .backfill_candidate(id)
            .await
            .expect("look up")
            .is_some(),
        "the user can still ask for the rest of it"
    );
}

// ---------------------------------------------------------------------------
// The payload axis (ADR 0017, #377)
// ---------------------------------------------------------------------------

/// A message carrying one named payload part, text already local.
fn with_a_payload(mailbox: MailboxId, account: postio_model::AccountId, seconds: i64) -> Message {
    let mut message = a_message(mailbox, account, seconds);
    message.sync.body_state = BodyState::Partial;
    message.content_type = Some("multipart/mixed".to_owned());
    let mut part = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 1_024);
    part.filename = Some("notes.pdf".to_owned());
    part.part_id = Some("2".to_owned());
    part.part_headers = Some("Content-Type: application/pdf\r\n".to_owned());
    message.attachments = vec![part];
    message
}

#[tokio::test]
async fn a_fetched_payload_is_recorded_against_the_part_that_asked_for_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = with_a_payload(inbox, account.id, 400);
    let id = messages.create(&mut message).await.expect("create");

    let blob = postio_model::BlobId::new("a".repeat(64));
    assert!(
        messages
            .set_attachment_blob(id, "2", &blob)
            .await
            .expect("write the key"),
        "the part is there to write against"
    );

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.attachments[0].blob_id, Some(blob));
    assert!(stored.attachments[0].is_downloaded());
}

#[tokio::test]
async fn a_payload_key_for_a_part_the_message_does_not_have_writes_nothing() {
    // The `part_id` the reading pane carries survives a refetch; an id from a
    // structure the server has since changed does not. Answering `false`
    // rather than erroring is what lets the caller treat it as "that part is
    // gone" -- the same answer `Outcome::Gone` gives for a whole message.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = with_a_payload(inbox, account.id, 401);
    let id = messages.create(&mut message).await.expect("create");

    let blob = postio_model::BlobId::new("b".repeat(64));
    assert!(
        !messages
            .set_attachment_blob(id, "7.3", &blob)
            .await
            .expect("ask")
    );

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.attachments[0].blob_id, None);
}

#[tokio::test]
async fn what_explains_a_payloads_bytes_is_kept_beside_it() {
    // Same reason `text_part_headers` exists (#376): `BODY[2]` hands back a
    // part's *encoded* bytes and none of its headers, so nothing in the
    // response says whether they are base64. `BODYSTRUCTURE` said so at
    // header-sync time and this is where the answer is kept.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = with_a_payload(inbox, account.id, 402);
    message.attachments[0].part_headers =
        Some("Content-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n".to_owned());
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(
        stored.attachments[0].part_headers.as_deref(),
        Some("Content-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n"),
    );
}

#[tokio::test]
async fn the_payload_backlog_is_the_partial_messages_still_missing_bytes() {
    // What `eager` drains. The background *text* lane treats `partial` as
    // settled (#376); the payload lane is the one that has work left there.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut wanted = with_a_payload(inbox, account.id, 403);
    let wanted_id = messages.create(&mut wanted).await.expect("create");

    let mut done = with_a_payload(inbox, account.id, 404);
    done.attachments[0].blob_id = Some(postio_model::BlobId::new("c".repeat(64)));
    messages.create(&mut done).await.expect("create");

    let mut no_text_yet = a_message(inbox, account.id, 405);
    no_text_yet.sync.body_state = BodyState::HeadersOnly;
    messages.create(&mut no_text_yet).await.expect("create");

    let candidates = messages
        .needing_payloads_from(inbox, 10, 0)
        .await
        .expect("candidates");

    assert_eq!(
        candidates.iter().map(|c| c.message_id).collect::<Vec<_>>(),
        vec![wanted_id],
        "only the message with a payload still on the server"
    );
}

#[tokio::test]
async fn a_message_whose_payloads_have_all_landed_becomes_full() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = with_a_payload(inbox, account.id, 406);
    let id = messages.create(&mut message).await.expect("create");

    messages
        .set_attachment_blob(id, "2", &postio_model::BlobId::new("d".repeat(64)))
        .await
        .expect("write the key");
    messages
        .set_body_state(id, BodyState::Full)
        .await
        .expect("settle it");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.sync.body_state, BodyState::Full);
    assert!(
        messages
            .needing_payloads_from(inbox, 10, 0)
            .await
            .expect("candidates")
            .is_empty(),
        "nothing left to fetch for it"
    );
}

#[tokio::test]
async fn two_messages_from_the_same_sender_share_one_address_row() {
    // `recipients` and its indexes are 56 MB of a 163 MB database -- 34%, and
    // larger than `messages` itself -- because 378,819 rows each store an
    // address and its lowercased near-duplicate. An account corresponds with
    // tens of thousands of distinct addresses, not 378,819 (ADR 0017).
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    for uid in [400, 401, 402] {
        let mut message = a_message(inbox, account.id, uid);
        // Exactly two correspondents, the same two every time -- the shape a
        // real mailbox has at scale, where a few thousand people account for
        // hundreds of thousands of header rows.
        message.from = vec![postio_model::EmailAddress::new(
            None::<String>,
            "ada@example.com",
        )];
        message.to = vec![postio_model::EmailAddress::new(
            None::<String>,
            "grace@example.com",
        )];
        message.cc.clear();
        messages.create(&mut message).await.expect("create");
    }

    let addresses: i64 =
        postio_storage::sql::one(&connection, "SELECT count(*) FROM addresses", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("count");
    let recipients: i64 =
        postio_storage::sql::one(&connection, "SELECT count(*) FROM recipients", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("count");

    assert_eq!(recipients, 6, "three messages, two addresses each");
    assert_eq!(addresses, 2, "but only two distinct addresses stored");
}

#[tokio::test]
async fn an_address_is_shared_case_insensitively() {
    // The point of `address_normalized` in the first place: `Ada@Example.com`
    // and `ada@example.com` are one correspondent, and `from:` has always
    // matched them as one. Sharing a row is what makes that structural rather
    // than a rule every query has to remember.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    for (uid, spelling) in [(410, "Ada@Example.com"), (411, "ada@example.com")] {
        let mut message = a_message(inbox, account.id, uid);
        message.from = vec![postio_model::EmailAddress::new(None::<String>, spelling)];
        message.to.clear();
        message.cc.clear();
        messages.create(&mut message).await.expect("create");
    }

    let addresses: i64 = postio_storage::sql::one(
        &connection,
        "SELECT count(*) FROM addresses WHERE address_normalized = 'ada@example.com'",
        (),
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("count");
    assert_eq!(addresses, 1, "one correspondent, however it was spelled");

    // And the row that exists keeps the first spelling seen, rather than
    // flip-flopping as later mail arrives.
    let stored: String = postio_storage::sql::one(
        &connection,
        "SELECT address FROM addresses WHERE address_normalized = 'ada@example.com'",
        (),
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the row");
    assert_eq!(stored, "Ada@Example.com");
}

#[tokio::test]
async fn a_message_still_reads_back_the_addresses_it_was_given() {
    // The normalization must be invisible above the repository: the verbatim
    // spelling and the display name are per-header facts and stay on the
    // recipient row, while only the addr-spec is shared.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 420);
    message.from = vec![postio_model::EmailAddress::new(
        Some("Ada Lovelace"),
        "Ada@Example.com",
    )];
    message.to = vec![
        postio_model::EmailAddress::new(Some("Grace Hopper"), "grace@example.com"),
        postio_model::EmailAddress::new(None::<String>, "katherine@example.com"),
    ];
    let id = messages.create(&mut message).await.expect("create");

    let stored = messages.get(id).await.expect("get").expect("the message");
    assert_eq!(stored.from, message.from, "verbatim spelling and name kept");
    assert_eq!(stored.to, message.to, "and header order preserved");
}

// ---------------------------------------------------------------------------
// A queued flag survives the resync that has not seen it yet (#317)
// ---------------------------------------------------------------------------
//
// The move case above drops the whole message from the batch, because the
// message is not in that mailbox any more as far as the user is concerned. A
// flag cannot be handled that way: the row *is* still there, and everything
// else the server says about it — subject, size, what parts it has — is news
// worth taking. Only the flags the queue is still holding intent about have to
// survive.

/// The shared fixture, but unread — which is the state a message the cursor
/// has not rested on is in, and the one these tests are about.
fn an_unread_message(
    mailbox: MailboxId,
    account: postio_model::AccountId,
    seconds: i64,
) -> Message {
    let mut message = a_message(mailbox, account, seconds);
    message.flags.remove(&postio_model::Flag::Seen);
    message
}

/// Marks `message` read locally and queues the flag, in the order the dwell
/// does it: the local write first, then the operation that will carry it.
async fn read_locally_and_enqueue(
    connection: &Connection,
    account: postio_model::AccountId,
    message: MessageId,
) {
    use postio_storage::repository::OperationQueueRepository;
    let mut flags = postio_model::FlagSet::new();
    flags.insert(postio_model::Flag::Seen);
    MessageRepository::new(connection)
        .set_flags(message, &flags, FlagSource::Local)
        .await
        .expect("the local write");
    OperationQueueRepository::new(connection)
        .enqueue(
            account,
            postio_model::OperationTarget::Message(message),
            &postio_model::Operation::SetFlags { flags },
            at(0),
        )
        .await
        .expect("enqueue");
}

async fn is_seen(connection: &Connection, message: MessageId) -> bool {
    MessageRepository::new(connection)
        .get(message)
        .await
        .expect("read")
        .expect("the message")
        .flags
        .contains(&postio_model::Flag::Seen)
}

#[tokio::test]
async fn a_resync_does_not_unread_a_message_whose_flag_has_not_drained() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    // A message the server has, unread, synced normally.
    let mut batch = vec![an_unread_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
    let message = batch[0].id;
    let (uid, validity) = (
        batch[0].server.uid.expect("uid"),
        batch[0].server.uid_validity.expect("validity"),
    );
    assert!(!is_seen(&connection, message).await, "it starts unread");

    // The cursor rests on it: read locally, and queued for the server.
    read_locally_and_enqueue(&connection, account.id, message).await;
    assert!(is_seen(&connection, message).await, "the dwell wrote it");

    // A CHANGEDSINCE pass runs before the drainer gets there. The server has
    // not been told yet, so it hands back exactly what it still believes.
    let mut resynced = vec![an_unread_message(inbox, account.id, 40)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    resynced[0].server.remote_id = Some(postio_model::RemoteId::new(format!("{validity}:{uid}")));
    messages
        .upsert_batch(&mut resynced)
        .await
        .expect("resync upsert");

    assert!(
        is_seen(&connection, message).await,
        "the message went bold again: a resync wrote the server's stale flags \
         over a local read the server has not heard about yet, so the dwell is \
         silently undone and the queued operation ends up setting a \\Seen \
         nobody can see (#317)"
    );
}

#[tokio::test]
async fn a_resync_still_takes_the_flags_the_queue_is_not_holding() {
    // The other half, and the reason this cannot be solved by skipping the
    // message the way an undrained move is. A flag the user never touched is
    // the server's to report -- somebody flagged it on their phone -- and it
    // has to arrive even while a *different* flag is mid-flight.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![an_unread_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
    let message = batch[0].id;
    let (uid, validity) = (
        batch[0].server.uid.expect("uid"),
        batch[0].server.uid_validity.expect("validity"),
    );

    read_locally_and_enqueue(&connection, account.id, message).await;

    // The server reports it flagged -- and still unseen, because it has not
    // heard about the read yet.
    let mut resynced = vec![an_unread_message(inbox, account.id, 40)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    resynced[0].server.remote_id = Some(postio_model::RemoteId::new(format!("{validity}:{uid}")));
    resynced[0].flags.insert(postio_model::Flag::Flagged);
    messages
        .upsert_batch(&mut resynced)
        .await
        .expect("resync upsert");

    let stored = messages
        .get(message)
        .await
        .expect("read")
        .expect("the message");
    assert!(
        stored.flags.contains(&postio_model::Flag::Flagged),
        "a flag set elsewhere never arrived: preserving local intent must not \
         mean refusing the server's news about everything else"
    );
    assert!(
        stored.flags.contains(&postio_model::Flag::Seen),
        "and the undrained read is still there"
    );
}

#[tokio::test]
async fn a_drained_flag_stops_being_protected() {
    // The bound on the rule: local intent wins only until the operation that
    // carries it has settled. After that the server is authoritative again,
    // and a message someone marked unread on their phone must be able to come
    // back unread here.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![an_unread_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
    let message = batch[0].id;
    let (uid, validity) = (
        batch[0].server.uid.expect("uid"),
        batch[0].server.uid_validity.expect("validity"),
    );

    read_locally_and_enqueue(&connection, account.id, message).await;

    // The drainer pushes it and marks the row done.
    {
        use postio_storage::repository::OperationQueueRepository;
        let queue = OperationQueueRepository::new(&connection);
        let pending = queue.pending(account.id, at(1)).await.expect("pending");
        for row in pending {
            queue.mark_done(row.id, at(2)).await.expect("settle");
        }
    }

    // Now the server says unseen -- which now means somebody read it and
    // marked it unread again elsewhere, not that our write is in flight.
    let mut resynced = vec![an_unread_message(inbox, account.id, 40)];
    resynced[0].server.uid = Some(uid);
    resynced[0].server.uid_validity = Some(validity);
    resynced[0].server.remote_id = Some(postio_model::RemoteId::new(format!("{validity}:{uid}")));
    messages
        .upsert_batch(&mut resynced)
        .await
        .expect("resync upsert");

    assert!(
        !is_seen(&connection, message).await,
        "a settled operation goes on protecting the flag it carried, so the \
         server can never mark anything unread again"
    );
}

#[tokio::test]
async fn upsert_matches_a_row_by_identity_before_the_wire_pair() {
    // #544: a non-IMAP backend's uid is a synthetic enumeration hint and can
    // shift between passes; the identity is what names the message. A fetch
    // carrying a known remote_id under a different uid must update the row
    // it names, never insert a second copy of the same message.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![a_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
    let id = batch[0].id;

    let mut shifted = vec![a_message(inbox, account.id, 40)];
    shifted[0].server.uid = Some(Uid::new(999));
    let report = messages
        .upsert_batch(&mut shifted)
        .await
        .expect("second pass");

    assert_eq!(report.updated, 1, "{report:?}");
    assert_eq!(report.inserted, 0, "{report:?}");
    assert_eq!(shifted[0].id, id, "the identity resolved to the same row");
}

#[tokio::test]
async fn a_truncated_header_block_says_so_when_it_is_read_back() {
    // The flag is the difference between "this message has no such header" and
    // "the part of it that was kept has none". An evaluator that could not
    // tell those apart would report absence with the same confidence either
    // way, which is the one thing a search must not do.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    let id = messages.create(&mut message).await.expect("create");

    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("the body".to_owned()),
                html: None,
                headers: Some("X-Mailer: mutt".to_owned()),
                headers_truncated: true,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("set");

    let stored = messages.body(id).await.expect("body").expect("the row");
    assert_eq!(stored.headers.as_deref(), Some("X-Mailer: mutt"));
    assert!(
        stored.headers_truncated,
        "the row lost the fact that its block was cut short"
    );
}

#[tokio::test]
async fn a_whole_header_block_is_not_marked_truncated() {
    // The ordinary case, and the one that must not drift to `true` by
    // accident: every message in a real store takes this path.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    let id = messages.create(&mut message).await.expect("create");

    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("the body".to_owned()),
                html: None,
                headers: Some("X-Mailer: mutt".to_owned()),
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("set");

    assert!(
        !messages
            .body(id)
            .await
            .expect("body")
            .expect("the row")
            .headers_truncated
    );
}

#[tokio::test]
async fn the_stored_block_comes_back_as_headers_rather_than_being_parsed_and_dropped() {
    // `ParsedMessage::into_message` has always filled `Message.headers`, and
    // the repository has never read or written them -- so a `Message` loaded
    // from the store came back with an empty block however much mail was in
    // it. That asymmetry is what #479's differential test exists to catch: the
    // in-memory matcher and the index have to be looking at the same headers.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    let id = messages.create(&mut message).await.expect("create");

    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("the body".to_owned()),
                html: None,
                headers: Some(
                    concat!(
                        "Received: from a.example.com\r\n",
                        // No leading whitespace on these: a continuation line
                        // is exactly how RFC 5322 folds one value across two
                        // lines, so an indented "X-Mailer:" here would be part
                        // of the Received above rather than a field of its own.
                        // rustfmt joining a line-continuation literal is what
                        // made that mistake the first time.
                        "X-Mailer: mutt 1.5.24\r\n",
                        "Received: from b.example.com",
                    )
                    .to_owned(),
                ),
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("set");

    let headers = messages
        .headers(id)
        .await
        .expect("headers")
        .expect("the row");

    assert_eq!(headers.get("x-mailer"), Some("mutt 1.5.24"));
    assert_eq!(
        headers.get_all("received").len(),
        2,
        "a hop chain is the reason duplicates are kept at all"
    );
    assert!(
        headers.contains("X-MAILER"),
        "RFC 5322 field names are case-insensitive on the way in too"
    );
}

#[tokio::test]
async fn a_message_with_no_stored_block_has_no_headers_rather_than_an_error() {
    // Every message in every store today, until the repair pass reaches it.
    // "Nothing downloaded yet" is not a fault.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    let id = messages.create(&mut message).await.expect("create");

    let headers = messages
        .headers(id)
        .await
        .expect("headers")
        .expect("the row");

    assert!(headers.is_empty());
}

#[tokio::test]
async fn a_fetched_message_with_no_stored_block_is_offered_for_repair() {
    // Every message in every store that exists today: `body_headers` has been
    // NULL since migration 0001 because nothing ever wrote it. The pass has to
    // find them, and has to stop finding them once they are done or it spins
    // (#500).
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut fetched = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    fetched.raw_blob_id = Some(postio_model::BlobId::new("a".repeat(64)));
    let fetched_id = messages.create(&mut fetched).await.expect("create");
    messages
        .set_body(
            fetched_id,
            &StoredBody {
                text: Some("the body".to_owned()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("set");

    // Never downloaded: not a repair candidate. Its block will arrive with its
    // body like any other, and offering it here would put a message on the
    // queue that the pass can do nothing about.
    let mut untouched = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    messages.create(&mut untouched).await.expect("create");

    let candidates = messages
        .messages_missing_headers(10)
        .await
        .expect("candidates");

    assert_eq!(candidates.len(), 1, "got: {candidates:?}");
    assert_eq!(candidates[0].message_id, fetched_id);
    assert!(
        candidates[0].raw_blob_id.is_some(),
        "the blob is what makes this repairable without a network call"
    );

    // And once repaired it stops coming back, which is the contract #500's
    // no-progress guard is watching for.
    messages
        .set_headers(
            fetched_id,
            Some(&postio_model::headers::Block {
                text: "X-Mailer: mutt".to_owned(),
                truncated: false,
            }),
        )
        .await
        .expect("repair");
    assert!(
        messages
            .messages_missing_headers(10)
            .await
            .expect("candidates")
            .is_empty()
    );
}

#[tokio::test]
async fn writing_a_repaired_block_leaves_the_body_beside_it_readable() {
    // The hazard this was written for is gone, and the claim it protects is
    // not. `body_text`, `body_html` and `body_headers` shared one
    // `body_dictionary_id`, so a repair that compressed the block against a
    // newer dictionary made the text and html beside it unreadable -- ADR
    // 0020's frames can only be read with the dictionary they were written
    // against. Losing a message's words to a pass that was only supposed to
    // add its headers is the shape of it.
    //
    // The columns are plain TEXT now (`specs/004-turso-store`), so there is
    // no dictionary to get wrong. What survives is the simpler half:
    // `set_headers` writes one column and must not touch the other two. That
    // is still a real way to lose mail -- routing the repair through
    // `set_body`, which replaces every part, would clear both -- and it is
    // what this now holds down.
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    let id = messages.create(&mut message).await.expect("create");

    let words = "the difference engine's seventh column".to_owned();
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(words.clone()),
                html: Some("<p>and the html</p>".to_owned()),
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .expect("store the body");

    messages
        .set_headers(
            id,
            Some(&postio_model::headers::Block {
                text: "X-Mailer: mutt".to_owned(),
                truncated: true,
            }),
        )
        .await
        .expect("repair");

    let stored = messages.body(id).await.expect("body").expect("the row");
    assert_eq!(
        stored.text.as_deref(),
        Some(words.as_str()),
        "the repair took the message's words with it"
    );
    assert_eq!(stored.html.as_deref(), Some("<p>and the html</p>"));
    assert_eq!(stored.headers.as_deref(), Some("X-Mailer: mutt"));
    assert!(stored.headers_truncated);
}

#[tokio::test]
async fn a_fetched_message_adopts_the_local_copy_we_wrote_before_the_server_had_one() {
    // #942. A sent message is written into Sent locally the moment it is on
    // its way, so the user can see it — before the IMAP `APPEND` has given it
    // a server identity. The identity arrives afterwards.
    //
    // Until then the row has no `remote_id` and no uid, so neither of the two
    // things `upsert_batch` matches on can find it: a resync of Sent that
    // fetched the server's own copy inserted a *second* row, and the user's
    // Sent folder showed the message twice.
    //
    // The reserved `Message-ID` is what ties them together. It is the same key
    // ADR 0021 already uses to answer "did this send arrive?" against the Sent
    // folder, and it is only consulted for a local row that has no server
    // identity at all — a row nothing but this client could have written.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let reserved = RfcMessageId::new("<reserved.once@example.com>");
    let mut ours = a_message(sent, account.id, 20);
    ours.rfc_message_id = Some(reserved.clone());
    ours.server = Default::default();
    messages.create(&mut ours).await.expect("the local copy");
    assert!(ours.server.remote_id.is_none(), "written before the append");

    // The server's copy of the same message, arriving through an ordinary
    // sync of the Sent folder.
    let mut fetched = a_message(sent, account.id, 21);
    fetched.rfc_message_id = Some(reserved.clone());
    let report = messages
        .upsert_batch(&mut vec![fetched.clone()])
        .await
        .expect("the resync");

    assert_eq!(
        report.inserted, 0,
        "the fetched copy was inserted beside the local row, so Sent now \
         shows the message twice"
    );
    assert_eq!(report.updated, 1, "it should have adopted the local row");

    let all = messages
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(sent),
            limit: 50,
            after: None,
        })
        .await
        .expect("a page");
    assert_eq!(
        all.len(),
        1,
        "Sent holds {} rows for one message: {:?}",
        all.len(),
        all.iter().map(|row| row.id).collect::<Vec<_>>()
    );

    let stored = messages
        .get(ours.id)
        .await
        .expect("get")
        .expect("the original row is the one that survived");
    assert_eq!(
        stored.server.remote_id, fetched.server.remote_id,
        "the local row did not take on the server identity, so the next \
         resync will insert the copy all over again"
    );
}

#[tokio::test]
async fn a_fetched_message_does_not_adopt_a_local_row_that_already_has_an_identity() {
    // The narrow half of the rule above. Adoption is only ever right for a
    // row this client wrote and the server has not yet named; a row that
    // already carries a `remote_id` is a different message that happens to
    // share a `Message-ID` — a mailing list copy of one's own post is the
    // ordinary case — and collapsing the two would lose one of them.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let shared = RfcMessageId::new("<shared@example.com>");
    let mut existing = a_message(inbox, account.id, 30);
    existing.rfc_message_id = Some(shared.clone());
    messages
        .create(&mut existing)
        .await
        .expect("an ordinary message");
    assert!(existing.server.remote_id.is_some());

    let mut fetched = a_message(inbox, account.id, 31);
    fetched.rfc_message_id = Some(shared);
    let report = messages
        .upsert_batch(&mut vec![fetched])
        .await
        .expect("the resync");

    assert_eq!(
        report.inserted, 1,
        "a message sharing a Message-ID with one that already has a server \
         identity is a different message and must not be collapsed into it"
    );
}

#[tokio::test]
async fn a_body_an_older_parser_got_wrong_is_fetched_again_once() {
    // A body is fetched once and its raw bytes are not kept, so a parser fix
    // reaches a stored body only by fetching it again -- and only the rows
    // the older parser got wrong: the ones that carried the decode caveat, or
    // came out with no body at all (three of them on a real account,
    // 2026-09-14). `messages.body_parsed_with` is what says which parser
    // wrote a row; `postio_model::mime::PARSER_VERSION` is the current one.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut flagged = a_message(inbox, account.id, 3);
    let mut empty = a_message(inbox, account.id, 2);
    let mut clean = a_message(inbox, account.id, 1);
    for message in [&mut flagged, &mut empty, &mut clean] {
        messages.create(message).await.expect("create");
    }
    let body = |text: Option<&str>, problems: bool| StoredBody {
        text: text.map(str::to_owned),
        html: None,
        headers: None,
        headers_truncated: false,
        encoding_problems: problems,
    };
    messages
        .set_body(
            flagged.id,
            &body(Some("=C3=A9 as six characters"), true),
            BodyState::Full,
        )
        .await
        .expect("set");
    messages
        .set_body(empty.id, &body(None, true), BodyState::Full)
        .await
        .expect("set");
    messages
        .set_body(clean.id, &body(Some("fine"), false), BodyState::Full)
        .await
        .expect("set");

    // Bodies a parser before this column wrote are stamped zero, whatever
    // they carry.
    connection
        .execute("UPDATE messages SET body_parsed_with = 0", ())
        .await
        .expect("age the rows");

    let again: Vec<MessageId> = messages
        .needing_backfill(inbox, 10)
        .await
        .expect("query")
        .into_iter()
        .map(|candidate| candidate.message_id)
        .collect();
    assert_eq!(
        again,
        [flagged.id, empty.id],
        "the two an older parser got wrong, newest first; a clean body is not \
         fetched again for a stamp alone"
    );

    // Fetched again by the current parser -- still flagged, say -- they are
    // done with: that is what keeps the backfill from spinning on a body no
    // parser can improve.
    messages
        .set_body(flagged.id, &body(Some("é"), true), BodyState::Full)
        .await
        .expect("set");
    messages
        .set_body(empty.id, &body(None, true), BodyState::Full)
        .await
        .expect("set");
    assert!(
        messages
            .needing_backfill(inbox, 10)
            .await
            .expect("query")
            .is_empty(),
        "a body the current parser wrote is never fetched again, caveat or not"
    );
}

#[tokio::test]
async fn a_stored_body_is_smaller_than_its_text_and_reads_back_whole() {
    // Bodies are text in a column (ADR 0020) and were zstd frames until the
    // engine changed, when the full-text index moved onto the column and an
    // index cannot tokenise compressed bytes. The index reads its own folded
    // table now (`message_search_bodies`), so the column is free to be small
    // again: the spec accepted a 2.19x store on the text axis, and it does
    // not have to.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 400);
    let id = messages.create(&mut message).await.expect("create");
    // A newsletter's worth of markup: what most of a store's bytes are.
    let html = "<tr><td class=\"cell\" style=\"padding:0 12px\">A line of the same shape as every other.</td></tr>\n".repeat(400);
    let text = "A line of plain text that reads much like the one before it.\n".repeat(200);
    let body = StoredBody {
        text: Some(text.clone()),
        html: Some(html.clone()),
        headers: Some("Subject: hello\r\n".to_owned()),
        headers_truncated: false,
        encoding_problems: false,
    };
    messages
        .set_body(id, &body, BodyState::Full)
        .await
        .expect("set");

    let stored = messages.body(id).await.expect("read").expect("the body");
    assert_eq!(stored, body, "what was written is what is read back");

    let (html_bytes, text_bytes): (i64, i64) = postio_storage::sql::one(
        &connection,
        "SELECT length(body_html), length(body_text) FROM messages WHERE id = ?1",
        [id.get()],
        |row| {
            use postio_storage::sql::RowExt;
            Ok((row.col(0)?, row.col(1)?))
        },
    )
    .await
    .expect("the column sizes");
    assert!(
        (html_bytes as usize) < html.len() / 4,
        "{html_bytes} bytes stored for {} bytes of markup: the column is not compressed",
        html.len()
    );
    assert!(
        (text_bytes as usize) < text.len() / 2,
        "{text_bytes} bytes stored for {} bytes of text",
        text.len()
    );
}

#[tokio::test]
async fn set_body_leaves_the_search_index_to_the_indexer() {
    // The body's full-text row used to be written here, in the body's own
    // transaction, so a search hit and the body it named could not come
    // apart. What that bought was paid on the sync lane: every body commit
    // updated the tantivy index, and whichever commit came next could
    // inherit a segment merge measured in seconds (`fts_merge_stall`). The
    // index is the indexer's now -- `postio_session::spawn_body_indexer`
    // batches hundreds of bodies into one write, off the sync lane -- and a
    // body is searchable a moment after it lands rather than in the same
    // instant, which is the trade every mail client makes.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut message = a_message(inbox, account.id, 500);
    let id = messages.create(&mut message).await.expect("create");
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("words worth finding".to_owned()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("set");
    let indexed = postio_storage::sql::exists(
        &connection,
        "SELECT 1 FROM message_search_bodies WHERE message_id = ?1",
        [id.get()],
    )
    .await
    .expect("ask the index");
    assert!(
        !indexed,
        "storing a body wrote its search row inside the body's transaction; \
         that write belongs to the indexer, off the sync lane"
    );
}
