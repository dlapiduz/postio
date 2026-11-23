use postio_storage::sql::bind;
use std::time::Duration;

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
    // #456: a reply built from a loaded message needs this to survive the round trip.
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

/// The body is stored on the row (ADR 0020) but not carried on the `Message` a `get` returns, or
/// the list would put the mailbox in memory.
#[tokio::test]
async fn a_read_message_does_not_carry_its_body_and_the_raw_blob_key_survives() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 5);
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

pub(crate) async fn seed(connection: &Connection, mailbox: MailboxId, count: u32) {
    connection
        .execute(
            "WITH RECURSIVE seq(n) AS (
                 SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < ?2
             )
             INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, subject, preview,
                                   flags, seen, flagged, size)
             SELECT (SELECT account_id FROM mailboxes WHERE id = ?1), ?1,
                    1770000000000 + n * 1000, 1770000000000 + n * 1000,
                    'Subject ' || n, 'Preview ' || n,
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
                    sort_at: at(0),
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

/// The nightly `paging_scale` test asks the clock; this asks the planner: an index term naming the
/// sort column is a seek. The cursor is written as a range, since this engine plans a row-value
/// comparison as a filter.
#[tokio::test]
async fn a_cursor_page_seeks_past_the_cursor_instead_of_filtering_down_to_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);

    for (label, query, sort_column) in [
        ("mailbox", ListQuery::mailbox(MailboxId::new(1)), "sort_at"),
        (
            "account",
            ListQuery::account(postio_model::AccountId::new(1)),
            "received_at",
        ),
    ] {
        let sql = messages.explain(&query.clone().after(ListCursor {
            received_at: at(0),
            sort_at: at(0),
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
async fn rows_for_answers_in_the_order_it_was_asked() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut ids = Vec::new();
    for step in 0..5 {
        let mut message = a_message(inbox, account.id, step * 10);
        messages.create(&mut message).await.expect("create");
        ids.push(message.id);
    }

    let ranked = vec![ids[3], ids[0], ids[4], ids[1]];
    let rows = messages.rows_for(&ranked).await.expect("rows");

    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        ranked,
        "the rows came back in the store's order rather than the ranking"
    );
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

    // A search can hand back a message deleted between the query and this read: a shorter answer,
    // not an error.
    messages.delete(&[ids[1]]).await.expect("delete");

    let rows = messages.rows_for(&ids).await.expect("rows");
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![ids[0], ids[2]],
        "a deleted hit was faked or the survivors were reordered"
    );

    // An empty `IN ()` is rejected by SQLite.
    assert!(messages.rows_for(&[]).await.expect("rows").is_empty());

    assert!(
        messages
            .rows_for(&[MessageId::new(999_999)])
            .await
            .expect("rows")
            .is_empty()
    );
}

// Archiving moves the row locally and queues a Move; an INBOX resync before the queue drains finds
// no row at the old (mailbox, validity, uid) and inserts a fresh one, resurrecting the message
// (#368).

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

    let mut batch = vec![a_message(inbox, account.id, 40)];
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
    assert_eq!(
        rows_in(&connection, inbox).await,
        0,
        "the archive was local-first"
    );
    assert_eq!(rows_in(&connection, archive.id).await, 1);

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

        // A server that still lists the message in INBOX means the move never happened or this is a
        // different message; either way the shadow must go.
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
    // NULL is the honest "not known" for rows already on disk; the backfill falls back to fetching
    // the whole message.
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
    // `partial` is a settled state for the background lane: asking for `body_state <> 'full'` made
    // the backfill spin on one message for ever and starve new mail.
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
    // `partial` is settled for the background lane, not the user: opening an attachment still needs
    // the lookup.
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
    // A `part_id` from a structure the server has since changed answers `false`, not an error, so
    // the caller treats the part as gone.
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
    // `BODY[2]` returns a part's encoded bytes with no headers, so whether they are base64 is kept
    // from `BODYSTRUCTURE` at header-sync time.
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    for uid in [400, 401, 402] {
        let mut message = a_message(inbox, account.id, uid);
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
    // `Ada@Example.com` and `ada@example.com` are one correspondent; sharing a row makes that
    // structural.
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

// A flag cannot be handled like an undrained move (dropping the message from the batch): the row is
// still there and the rest of what the server says is news. Only flags the queue still holds intent
// about must survive (#317).

fn an_unread_message(
    mailbox: MailboxId,
    account: postio_model::AccountId,
    seconds: i64,
) -> Message {
    let mut message = a_message(mailbox, account, seconds);
    message.flags.remove(&postio_model::Flag::Seen);
    message
}

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

    let mut batch = vec![an_unread_message(inbox, account.id, 40)];
    messages.upsert_batch(&mut batch).await.expect("first sync");
    let message = batch[0].id;
    let (uid, validity) = (
        batch[0].server.uid.expect("uid"),
        batch[0].server.uid_validity.expect("validity"),
    );
    assert!(!is_seen(&connection, message).await, "it starts unread");

    read_locally_and_enqueue(&connection, account.id, message).await;
    assert!(is_seen(&connection, message).await, "the dwell wrote it");

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
    // A flag the user never touched is the server's to report, even while a different flag is
    // mid-flight.
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
    // Local intent wins only until its operation has settled; after that the server is
    // authoritative.
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

    {
        use postio_storage::repository::OperationQueueRepository;
        let queue = OperationQueueRepository::new(&connection);
        let pending = queue.pending(account.id, at(1)).await.expect("pending");
        for row in pending {
            queue.mark_done(row.id, at(2)).await.expect("settle");
        }
    }

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
    // #544: a non-IMAP backend's uid is a synthetic hint that can shift; a known remote_id under a
    // different uid updates the row it names.
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
    // The flag separates "no such header" from "the kept part has none"; a search must not report
    // absence with the same confidence for both.
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
    // `into_message` fills `Message.headers` but the repository never read or wrote them, so the
    // in-memory matcher and the index saw different headers (#479).
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
                        // No leading whitespace: a continuation line folds one value across two
                        // lines, so an indented "X-Mailer:" would be part of the Received.
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
    // `body_headers` was NULL since migration 0001; the pass must find those rows and stop finding
    // them once done, or it spins (#500).
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
    // `set_headers` writes one column and must not touch the other two; routing the repair through
    // `set_body` would clear both.
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
    // #942: a sent message is written into Sent before its APPEND gives it a server identity, so a
    // resync of Sent inserted a second row. The reserved `Message-ID` (ADR 0021) ties them,
    // consulted only for a local row with no server identity.
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
    // Adoption is only right for a row the server has not named; a row with a `remote_id` is a
    // different message sharing a `Message-ID`, such as a list copy of one's own post.
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
    // Raw bytes are not kept, so a parser fix reaches a stored body only by refetching, and only
    // rows the older parser got wrong; `messages.body_parsed_with` says which parser wrote a row.
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

    // Fetched again by the current parser they are done, which keeps the backfill from spinning.
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
    // Bodies are plain text in a column (ADR 0020); the full-text index reads its own folded table,
    // so the column need not be a compressed frame.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = a_message(inbox, account.id, 400);
    let id = messages.create(&mut message).await.expect("create");
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
        "SELECT length(c.body_html), length(c.body_text) FROM messages m JOIN message_contents c ON c.id = m.content_id WHERE m.id = ?1",
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
    // The body's full-text row is the indexer's, not written in the body's transaction: a segment
    // merge would stall the sync lane (`fts_merge_stall`), so a body is searchable a moment after
    // it lands.
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
        "SELECT 1 FROM message_search_bodies WHERE content_id = (SELECT content_id FROM messages WHERE id = ?1)",
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
