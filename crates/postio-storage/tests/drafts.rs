//! Drafts: CRUD and the autosave-friendly upsert.
//!
//! The bead's acceptance criterion is "draft upsert is idempotent under rapid
//! autosave".

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Attachment, Draft, DraftId, DraftKind, DraftState, EmailAddress, Message, MessageBody,
    MessageId, Operation, OperationTarget, ThreadId,
};
use postio_storage::repository::{DraftRepository, MessageRepository, OperationQueueRepository};
use postio_storage::test_support;

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + chrono::Duration::minutes(minutes)
}

fn a_draft(account: postio_model::AccountId) -> Draft {
    let mut draft = Draft::new(account);
    draft.subject = "Tide gate interlock".to_owned();
    draft.to = vec![EmailAddress::new(Some("Quinn Abara"), "quinn@example.net")];
    draft.cc = vec![EmailAddress::new(None::<String>, "list@example.org")];
    draft.body = MessageBody {
        text: Some("Half a sentence".to_owned()),
        html: None,
    };
    draft.created_at = at(0);
    draft.updated_at = at(0);
    draft
}

// ---------------------------------------------------------------------------
// Create and read
// ---------------------------------------------------------------------------

#[test]
fn a_draft_round_trips_with_its_recipients_and_attachments() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::Reply;
    draft.bcc = vec![EmailAddress::new(None::<String>, "archive@example.com")];
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 1_024);
    attachment.filename = Some("revision-c.pdf".to_owned());
    draft.attachments = vec![attachment];

    let id = drafts.save(&mut draft).expect("save");

    assert!(id.is_assigned());
    assert_eq!(draft.id, id);
    assert!(draft.attachments[0].id.is_assigned());

    let stored = drafts.get(id).expect("get").expect("the draft");
    assert_eq!(stored.subject, "Tide gate interlock");
    assert_eq!(stored.to, draft.to);
    assert_eq!(stored.cc, draft.cc);
    assert_eq!(stored.bcc, draft.bcc);
    assert_eq!(stored.body, draft.body);
    assert_eq!(stored.kind, DraftKind::Reply);
    assert_eq!(stored.state, DraftState::Editing);
    assert_eq!(stored.attachments, draft.attachments);
    assert_eq!(stored.created_at, at(0));
    assert_eq!(stored.updated_at, at(0));
    assert!(stored.has_recipients() && stored.is_sendable());
}

#[test]
fn the_body_of_a_draft_is_stored_inline_and_not_in_the_blob_store() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.body.html = Some("<p>Half a sentence</p>".to_owned());
    let id = drafts.save(&mut draft).expect("save");

    let (text, html): (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT body_text, body_html FROM drafts WHERE id = ?1",
            [id.get()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read the raw row");

    assert_eq!(text.as_deref(), Some("Half a sentence"));
    assert_eq!(html.as_deref(), Some("<p>Half a sentence</p>"));
}

#[test]
fn reading_a_draft_that_is_not_there_is_none() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let drafts = DraftRepository::new(&connection);

    assert!(drafts.get(DraftId::new(404)).expect("get").is_none());
    assert!(!drafts.delete(DraftId::new(404)).expect("delete"));
}

// ---------------------------------------------------------------------------
// Acceptance: autosave is idempotent
// ---------------------------------------------------------------------------

#[test]
fn saving_the_same_draft_repeatedly_writes_one_row() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let id = drafts.save(&mut draft).expect("first save");

    // The composer autosaves on every keystroke.
    for keystroke in 1..=50 {
        draft.subject = format!("Tide gate interlock{}", "!".repeat(keystroke));
        draft.updated_at = at(keystroke as i64);
        let same = drafts.save(&mut draft).expect("autosave");
        assert_eq!(same, id, "autosave never starts a second draft");
    }

    for (table, expected) in [("drafts", 1), ("recipients", 2), ("attachments", 0)] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, expected, "{table} must not accumulate");
    }

    let stored = drafts.get(id).expect("get").expect("the draft");
    assert_eq!(stored.subject, draft.subject);
    assert_eq!(stored.updated_at, at(50), "but the timestamp moves");
    assert_eq!(stored.created_at, at(0), "and the start does not");
}

#[test]
fn autosave_keeps_attachment_identity_stable() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.attachments = vec![Attachment::new(MessageId::UNASSIGNED, "image/png", 64)];
    let id = drafts.save(&mut draft).expect("save");
    let attachment_id = draft.attachments[0].id;

    draft.body.text = Some("More text".to_owned());
    drafts.save(&mut draft).expect("autosave");

    let stored = drafts.get(id).expect("get").expect("the draft");
    assert_eq!(stored.attachments.len(), 1);
    assert_eq!(
        stored.attachments[0].id, attachment_id,
        "an attachment the user added keeps its id across autosaves"
    );
}

#[test]
fn removing_a_recipient_removes_the_row() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let id = drafts.save(&mut draft).expect("save");

    draft.cc.clear();
    drafts.save(&mut draft).expect("autosave");

    let stored = drafts.get(id).expect("get").expect("the draft");
    assert!(stored.cc.is_empty());
    assert_eq!(stored.to.len(), 1);
}

// ---------------------------------------------------------------------------
// The send queue and the composer's other reads
// ---------------------------------------------------------------------------

#[test]
fn drafts_list_most_recently_edited_first() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut older = a_draft(account.id);
    older.updated_at = at(1);
    let older_id = drafts.save(&mut older).expect("save");
    let mut newer = a_draft(account.id);
    newer.updated_at = at(2);
    let newer_id = drafts.save(&mut newer).expect("save");

    let listed: Vec<DraftId> = drafts
        .list_for_account(account.id)
        .expect("list")
        .iter()
        .map(|draft| draft.id)
        .collect();

    assert_eq!(listed, [newer_id, older_id]);
}

#[test]
fn the_send_queue_reads_drafts_by_state_in_the_order_they_were_queued() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut first = a_draft(account.id);
    first.updated_at = at(1);
    let first_id = drafts.save(&mut first).expect("save");
    let mut second = a_draft(account.id);
    second.updated_at = at(2);
    let second_id = drafts.save(&mut second).expect("save");
    let mut editing = a_draft(account.id);
    drafts.save(&mut editing).expect("save");

    drafts
        .set_state(first_id, DraftState::Queued)
        .expect("queue");
    drafts
        .set_state(second_id, DraftState::Queued)
        .expect("queue");

    let queued: Vec<DraftId> = drafts
        .by_state(DraftState::Queued)
        .expect("by state")
        .iter()
        .map(|draft| draft.id)
        .collect();

    assert_eq!(
        queued,
        [first_id, second_id],
        "oldest first: sending is a queue, not a stack"
    );
    assert!(
        !drafts
            .get(first_id)
            .expect("get")
            .expect("the draft")
            .is_sendable(),
        "a queued draft is no longer the composer's to send again"
    );
}

#[test]
fn a_reply_draft_remembers_the_message_and_thread_it_belongs_to() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let drafts = DraftRepository::new(&connection);

    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1)",
            [account.id.get()],
        )
        .expect("a thread");
    let mut parent = Message::new(account.id, inbox, at(0));
    parent.subject = Some("Tide gate interlock".to_owned());
    MessageRepository::new(&connection)
        .create(&mut parent)
        .expect("create");

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::ReplyAll;
    draft.in_reply_to = Some(parent.id);
    draft.thread_id = Some(ThreadId::new(1));
    let id = drafts.save(&mut draft).expect("save");

    let stored = drafts.get(id).expect("get").expect("the draft");
    assert_eq!(stored.in_reply_to, Some(parent.id));
    assert_eq!(stored.thread_id, Some(ThreadId::new(1)));
    assert_eq!(
        drafts.in_thread(ThreadId::new(1)).expect("in thread").len(),
        1,
        "the composer takes over the reading pane inside the thread"
    );
}

#[test]
fn a_draft_survives_the_message_it_replies_to_being_expunged() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut parent = Message::new(account.id, inbox, at(0));
    MessageRepository::new(&connection)
        .create(&mut parent)
        .expect("create");
    let mut draft = a_draft(account.id);
    draft.in_reply_to = Some(parent.id);
    let id = drafts.save(&mut draft).expect("save");

    MessageRepository::new(&connection)
        .delete(&[parent.id])
        .expect("expunge");

    let stored = drafts
        .get(id)
        .expect("get")
        .expect("the draft is still there");
    assert_eq!(
        stored.in_reply_to, None,
        "losing the parent must never lose what the user typed"
    );
}

#[test]
fn deleting_a_draft_takes_its_recipients_and_attachments() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.attachments = vec![Attachment::new(MessageId::UNASSIGNED, "image/png", 8)];
    let id = drafts.save(&mut draft).expect("save");

    assert!(drafts.delete(id).expect("delete"));

    for table in ["drafts", "recipients", "attachments"] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 0, "{table}");
    }
}

#[test]
fn enumerations_are_stored_with_the_spelling_the_model_documents() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::Forward;
    let id = drafts.save(&mut draft).expect("save");
    drafts.set_state(id, DraftState::Failed).expect("fail it");

    let (kind, state): (String, String) = connection
        .query_row(
            "SELECT kind, state FROM drafts WHERE id = ?1",
            [id.get()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read the raw row");

    assert_eq!(kind, DraftKind::Forward.as_str());
    assert_eq!(state, DraftState::Failed.as_str());
    assert!(
        drafts
            .get(id)
            .expect("get")
            .expect("the draft")
            .is_sendable(),
        "a failed draft is editable again"
    );
}

// ---------------------------------------------------------------------------
// Queueing the server copy
// ---------------------------------------------------------------------------
//
// A draft is durable on this machine the moment `save` returns, and reaches
// the account's Drafts mailbox later, through the same queue every other
// mutation goes through. These are the enqueue half; `postio-sync`'s
// `tests/drafts.rs` is the drain half.

/// An account with the Drafts mailbox a draft is filed into.
fn account_with_drafts(
    connection: &rusqlite::Connection,
) -> (postio_model::Account, postio_model::MailboxId) {
    let account = test_support::account(connection);
    let drafts = test_support::mailbox(connection, &account, "Drafts");
    assert_eq!(
        drafts.role,
        postio_model::MailboxRole::Drafts,
        "the fixture depends on the role being derived from the path"
    );
    (account, drafts.id)
}

#[test]
fn saving_a_draft_queues_it_for_the_server_in_the_same_write() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let queued = drafts
        .save_and_sync(&mut draft, at(0))
        .expect("save and queue")
        .expect("a queue row");

    assert_eq!(
        queued.operation,
        Operation::SaveDraft {
            mailbox: drafts_mailbox
        }
    );
    assert_eq!(queued.target, OperationTarget::Draft(draft.id));
    assert!(
        drafts.get(draft.id).expect("get").is_some(),
        "the local row is written whether or not the server ever hears about it"
    );
}

#[test]
fn a_draft_with_nowhere_to_go_is_still_saved_locally() {
    // No Drafts mailbox: the account has not been synced far enough to know
    // one exists. Local-first means the draft is kept anyway, and the next
    // save after the folder turns up is what files it.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let queued = drafts.save_and_sync(&mut draft, at(0)).expect("save");

    assert!(queued.is_none(), "there is no folder to file it in");
    assert!(drafts.get(draft.id).expect("get").is_some());
}

#[test]
fn discarding_a_draft_removes_it_locally_and_queues_the_server_copy() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.server.uid = Some(postio_model::Uid::new(41));
    draft.server.uid_validity = Some(postio_model::UidValidity::new(9));
    drafts.save(&mut draft).expect("save");

    let queued = drafts
        .discard(draft.id, at(1))
        .expect("discard")
        .expect("a queue row");

    assert_eq!(
        queued.operation,
        Operation::DiscardDraft {
            mailbox: drafts_mailbox,
            uid: postio_model::Uid::new(41),
            uid_validity: postio_model::UidValidity::new(9),
        },
        "the operation carries the copy to remove, because the row that knew \
         it is about to be gone"
    );
    assert!(
        drafts.get(draft.id).expect("get").is_none(),
        "the draft is gone here the moment the user says so"
    );
}

#[test]
fn discarding_a_draft_the_server_never_saw_queues_nothing() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");

    assert!(
        drafts.discard(draft.id, at(1)).expect("discard").is_none(),
        "nothing was uploaded, so there is nothing to remove"
    );
    assert!(drafts.get(draft.id).expect("get").is_none());
    let pending = OperationQueueRepository::new(&connection)
        .pending(account.id, at(2))
        .expect("pending");
    assert!(pending.is_empty(), "and no round trip is spent saying so");
}

#[test]
fn discarding_a_draft_that_is_already_gone_is_not_an_error() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_, _) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    assert!(
        drafts
            .discard(DraftId::new(404), at(1))
            .expect("discard")
            .is_none(),
        "a retried discard is the expected case, not a failure"
    );
}

#[test]
fn queueing_a_draft_for_sending_marks_it_and_enqueues_the_operation() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");

    let queued = drafts
        .queue_send(&mut draft, at(1))
        .expect("queue the send");

    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert_eq!(queued.target, OperationTarget::Draft(draft.id));
    assert_eq!(
        drafts
            .get(draft.id)
            .expect("get")
            .expect("the draft is still here")
            .state,
        DraftState::Queued,
        "the row has to survive the enqueue: `postio-sync::send` builds the \
         message's bytes from it when the operation drains, and resolves a \
         missing draft as obsolete"
    );
    assert_eq!(draft.state, DraftState::Queued, "and the caller's copy too");
}

#[test]
fn queueing_a_send_writes_the_draft_that_was_never_saved() {
    // Ctrl+Enter can beat the debounced autosave: a draft typed and sent
    // inside the quiet period has no row and no id yet. The enqueue names the
    // draft by id, so there is no send to queue until there is one.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    assert!(!draft.id.is_assigned(), "nothing has saved it yet");

    let queued = drafts
        .queue_send(&mut draft, at(1))
        .expect("queue the send");

    assert!(draft.id.is_assigned());
    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert!(drafts.get(draft.id).expect("get").is_some());
}

#[test]
fn a_queued_send_does_not_need_a_drafts_mailbox() {
    // Unlike `save_and_sync`, which has nowhere to file the draft until the
    // first sync finds the folder, a send names no mailbox at all: SMTP is a
    // different conversation from IMAP, and the Sent copy is resolved when the
    // operation drains rather than now.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let queued = drafts
        .queue_send(&mut draft, at(1))
        .expect("queue the send");

    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert!(queued.mailbox_id.is_none());
}

// ---------------------------------------------------------------------------
// The draft row and the message row are the same message
// ---------------------------------------------------------------------------
//
// A draft is appended to the account's Drafts mailbox, and the next sync pass
// over that folder fetches it straight back. Without this, the same unfinished
// message exists twice locally — once as the composer's `drafts` row and once
// as an ordinary `messages` row — and the second one is a read-only snapshot
// of a buffer that is still being typed into. See #51.
//
// The composer owns a draft this client wrote. Another client's draft has no
// local draft row and is the reason the folder is worth syncing at all, so it
// stays an ordinary message.

/// A message as sync would have built it: in `mailbox`, carrying the server
/// identity the fetch reported.
fn fetched(
    account: postio_model::AccountId,
    mailbox: postio_model::MailboxId,
    uid: u32,
    validity: u32,
) -> Message {
    let mut message = Message::new(account, mailbox, at(0));
    message.subject = Some("Tide gate interlock".to_owned());
    message.server.uid = Some(postio_model::Uid::new(uid));
    message.server.uid_validity = Some(postio_model::UidValidity::new(validity));
    message
}

/// A draft whose server copy is `uid` under `validity`.
fn uploaded(
    connection: &rusqlite::Connection,
    account: postio_model::AccountId,
    uid: u32,
    validity: u32,
) -> DraftId {
    let mut draft = a_draft(account);
    let drafts = DraftRepository::new(connection);
    let id = drafts.save(&mut draft).expect("save the draft");
    drafts
        .set_server_copy(
            id,
            Some(postio_model::Uid::new(uid)),
            Some(postio_model::UidValidity::new(validity)),
        )
        .expect("record where the append landed");
    id
}

fn rows_in(connection: &rusqlite::Connection, mailbox: postio_model::MailboxId) -> u32 {
    MessageRepository::new(connection)
        .count_set(&postio_storage::repository::MessageSet::in_mailbox(mailbox))
        .expect("a count")
}

#[test]
fn a_draft_this_client_uploaded_does_not_come_back_as_a_message() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts) = account_with_drafts(&connection);
    uploaded(&connection, account.id, 7, 1);

    let mut batch = vec![fetched(account.id, drafts, 7, 1)];
    let report = MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over Drafts");

    assert_eq!(report.inserted, 0);
    assert_eq!(report.own_drafts, 1);
    assert!(
        batch.is_empty(),
        "the batch is what the caller goes on to thread and record contacts \
         from, so a skipped message has to leave it"
    );
    assert_eq!(
        rows_in(&connection, drafts),
        1,
        "the one row is the one `save` wrote for the folder to list (#166); \
         the copy that came back down added nothing"
    );
}

#[test]
fn a_draft_written_by_another_client_is_an_ordinary_message() {
    // The reason the folder syncs at all. This one has no local draft row, so
    // there is nothing for it to be a second copy of.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts) = account_with_drafts(&connection);
    uploaded(&connection, account.id, 7, 1);

    let mut batch = vec![
        fetched(account.id, drafts, 7, 1),
        fetched(account.id, drafts, 8, 1),
    ];
    let report = MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over Drafts");

    assert_eq!(report.inserted, 1);
    assert_eq!(report.own_drafts, 1);
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].server.uid.map(postio_model::Uid::get), Some(8));
    assert_eq!(
        rows_in(&connection, drafts),
        2,
        "the draft's own row, and the other client's draft beside it"
    );
}

#[test]
fn a_uid_that_matches_a_draft_in_a_different_folder_is_an_ordinary_message() {
    // UIDs are per-mailbox, so message 7 in INBOX has nothing to do with the
    // draft that is message 7 in Drafts. Matching on the number alone would
    // hide a piece of the user's mail.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _drafts) = account_with_drafts(&connection);
    let inbox = test_support::mailbox(&connection, &account, "INBOX").id;
    uploaded(&connection, account.id, 7, 1);

    let mut batch = vec![fetched(account.id, inbox, 7, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over INBOX");

    assert_eq!(rows_in(&connection, inbox), 1);
}

#[test]
fn a_draft_whose_append_was_never_located_hides_nothing() {
    // No `UIDPLUS`, so `save` recorded that it does not know where the copy
    // landed and flagged the folder for a resync instead. There is nothing to
    // match on, and guessing which message in Drafts is ours is exactly what
    // `postio-sync`'s draft module refuses to do.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts) = account_with_drafts(&connection);
    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save the draft");

    let mut batch = vec![fetched(account.id, drafts, 7, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over Drafts");

    assert_eq!(
        rows_in(&connection, drafts),
        2,
        "the draft's own row, and the copy of it that came back down — \
         nothing links the two, which is what `postio-sync::drafts` flags the \
         folder for a resync over rather than guessing about"
    );
}

#[test]
fn a_draft_recorded_under_an_older_generation_hides_nothing() {
    // A renumbered mailbox makes the old UID name some other message, which
    // is the same reason `discard` carries its `UIDVALIDITY`.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts) = account_with_drafts(&connection);
    uploaded(&connection, account.id, 7, 1);

    let mut batch = vec![fetched(account.id, drafts, 7, 2)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over a renumbered Drafts");

    assert_eq!(
        rows_in(&connection, drafts),
        2,
        "the draft's own row, and the message that is number 7 under the new \
         generation — which is not the same message at all"
    );
}

#[test]
fn a_message_row_that_beat_the_draft_to_its_uid_is_taken_back_out() {
    // The race the skip alone does not close: a sync pass fetched the
    // appended copy before `set_server_copy` had recorded where it landed, so
    // the row was already there when the draft learned its own UID. Every
    // later pass would then find the row and update it, and the duplicate
    // would be permanent. Claiming the copy is therefore also what removes it.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts) = account_with_drafts(&connection);
    let elsewhere = test_support::mailbox(&connection, &account, "INBOX").id;
    let mut first = vec![
        fetched(account.id, drafts, 7, 1),
        fetched(account.id, elsewhere, 7, 1),
    ];
    MessageRepository::new(&connection)
        .upsert_batch(&mut first)
        .expect("a pass that ran before the draft was linked");
    assert_eq!(
        rows_in(&connection, drafts),
        1,
        "the duplicate this repairs"
    );

    uploaded(&connection, account.id, 7, 1);

    assert_eq!(
        rows_in(&connection, drafts),
        1,
        "the stray row goes and the draft's own row is what is left, rather \
         than the two of them sitting side by side"
    );
    assert_eq!(
        rows_in(&connection, elsewhere),
        1,
        "UIDs are per-mailbox; the inbox message that happens to be number 7 \
         is somebody's mail"
    );
}

// ---------------------------------------------------------------------------
// A draft's place in the Drafts folder
// ---------------------------------------------------------------------------
//
// #51 stopped the synced copy of a draft becoming a second message row, which
// left the Drafts folder listing other clients' drafts and nothing else — and
// the sidebar badge, which reads the mailbox's cached count of message rows,
// saying 0 while the composer held a draft. #166.
//
// A draft's list presence is therefore a `messages` row this repository writes,
// not one sync brings back. That is the only source that is right immediately:
// a draft has no server copy until an append has round-tripped, and a folder
// that only listed your draft after a network exchange is exactly what
// docs/PRODUCT.md §18's local-first rule forbids.

/// What the message list would show for `mailbox`, subject first.
fn folder(connection: &rusqlite::Connection, mailbox: postio_model::MailboxId) -> Vec<String> {
    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(mailbox),
        limit: 50,
        after: None,
    };
    MessageRepository::new(connection)
        .page(&query)
        .expect("a page")
        .into_iter()
        .map(|row| row.subject.unwrap_or_default())
        .collect()
}

/// The count the sidebar draws under "Drafts".
fn badge(connection: &rusqlite::Connection, mailbox: postio_model::MailboxId) -> u32 {
    postio_storage::repository::MailboxRepository::new(connection)
        .counts(mailbox)
        .expect("a read")
        .expect("the mailbox")
        .total
}

#[test]
fn saving_a_draft_puts_it_in_the_drafts_folder_at_once() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);

    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save the draft");

    assert_eq!(
        folder(&connection, drafts_mailbox),
        vec!["Tide gate interlock".to_owned()],
        "no server round trip stands between typing and this"
    );
    assert_eq!(badge(&connection, drafts_mailbox), 1);
}

#[test]
fn the_row_a_draft_owns_is_marked_as_a_draft_and_as_read() {
    // The list already draws a draft mark and says "Draft" in the accessible
    // label off `MessageListRow::draft`; unread is for mail that arrived.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);

    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("save the draft");

    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(drafts_mailbox),
        limit: 50,
        after: None,
    };
    let rows = MessageRepository::new(&connection)
        .page(&query)
        .expect("a page");
    assert!(rows[0].draft, "the row the folder shows is a draft");
    assert!(rows[0].seen, "your own draft is not unread mail");
    assert_eq!(badge(&connection, drafts_mailbox), 1);
}

#[test]
fn autosave_keeps_one_row_and_keeps_it_current() {
    // The autosave rule this repository already holds for the draft row, held
    // for its list row too: a keystroke is not a new draft.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");
    draft.subject = "Tide gate interlock, revised".to_owned();
    draft.updated_at = at(5);
    drafts.save(&mut draft).expect("save again");

    assert_eq!(
        folder(&connection, drafts_mailbox),
        vec!["Tide gate interlock, revised".to_owned()]
    );
    assert_eq!(badge(&connection, drafts_mailbox), 1);
}

#[test]
fn discarding_a_draft_takes_its_row_out_of_the_folder() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");
    drafts.discard(draft.id, at(5)).expect("discard");

    assert!(folder(&connection, drafts_mailbox).is_empty());
    assert_eq!(badge(&connection, drafts_mailbox), 0);
}

#[test]
fn sending_a_draft_takes_its_row_out_of_the_folder() {
    // `postio-sync::send` finishes by deleting the draft, which is the single
    // exit both it and discard go through.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");
    assert!(drafts.delete(draft.id).expect("delete"));

    assert!(folder(&connection, drafts_mailbox).is_empty());
    assert_eq!(badge(&connection, drafts_mailbox), 0);
}

#[test]
fn an_account_with_no_drafts_folder_yet_still_saves_the_draft() {
    // The ordinary state of an account that has not finished its first sync.
    // The draft is durable regardless; it simply has nowhere to be listed.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);

    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("a draft is durable before the folder exists");

    assert!(draft.id.is_assigned());
}

#[test]
fn the_row_a_draft_owns_is_the_one_its_server_copy_attaches_to() {
    // The two halves have to meet. The append lands, `set_server_copy` records
    // where — and the row the folder is already showing becomes the row that
    // names that copy, rather than a second one appearing beside it.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");
    drafts
        .set_server_copy(
            draft.id,
            Some(postio_model::Uid::new(7)),
            Some(postio_model::UidValidity::new(1)),
        )
        .expect("record where the append landed");

    assert_eq!(folder(&connection, drafts_mailbox).len(), 1);

    // And the sync pass that fetches the copy back still adds nothing: #51.
    let mut batch = vec![fetched(account.id, drafts_mailbox, 7, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over Drafts");

    assert_eq!(
        folder(&connection, drafts_mailbox),
        vec!["Tide gate interlock".to_owned()],
        "one draft, one row, whichever half wrote it"
    );
    assert_eq!(badge(&connection, drafts_mailbox), 1);
}

#[test]
fn a_drafts_row_leads_back_to_the_draft_it_is_listing() {
    // The link the other way. The message list hands back a `MessageId`, and
    // activating a draft's row has to reach the buffer the composer edits —
    // opening the reader on it instead is the dead end #166 is about.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let drafts = DraftRepository::new(&connection);
    // Mail in the folder first, so the draft's row does not land on the same
    // number as the draft. Without this the assertion below holds for a
    // `by_message` that looked up the draft by its own id, and the test could
    // not tell the link from the coincidence.
    let mut noise = vec![
        fetched(account.id, drafts_mailbox, 30, 1),
        fetched(account.id, drafts_mailbox, 31, 1),
        fetched(account.id, drafts_mailbox, 32, 1),
    ];
    MessageRepository::new(&connection)
        .upsert_batch(&mut noise)
        .expect("three drafts written elsewhere");

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).expect("save");
    let listed = MessageRepository::new(&connection)
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(drafts_mailbox),
            limit: 50,
            after: None,
        })
        .expect("a page");
    let row = listed
        .iter()
        .find(|row| row.subject.as_deref() == Some("Tide gate interlock"))
        .expect("the draft is in the folder");
    assert_ne!(
        row.id.get(),
        draft.id.get(),
        "the fixture exists to make these differ"
    );

    let found = drafts
        .by_message(row.id)
        .expect("a read")
        .expect("the row is a draft's, so there is one");

    assert_eq!(found.id, draft.id);
    assert_eq!(found.subject, draft.subject);
    assert_eq!(
        found.to, draft.to,
        "the whole draft, recipients and all — the composer opens on this"
    );
}

#[test]
fn a_message_that_is_not_a_drafts_row_leads_nowhere() {
    // Another client's draft, which has no local buffer to open. What happens
    // then is `postio-app`'s decision; what is certain here is that there is
    // nothing to find — including when its row number happens to be a draft's
    // id, which is the coincidence a link keyed on the wrong column survives.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection);
    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .expect("a draft, so there is an id to collide with");

    let mut batch = vec![fetched(account.id, drafts_mailbox, 9, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .expect("a sync pass over Drafts");
    let foreign = batch[0].id;

    assert!(
        DraftRepository::new(&connection)
            .by_message(foreign)
            .expect("a read")
            .is_none()
    );
    // And the one that *is* a draft's still leads to it, so the assertion
    // above is about the link rather than about `by_message` finding nothing.
    let mine = DraftRepository::new(&connection)
        .get(draft.id)
        .expect("a read")
        .expect("the draft");
    assert_eq!(mine.id, draft.id);
}
