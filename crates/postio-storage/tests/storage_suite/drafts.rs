//! Drafts: CRUD and the autosave-friendly upsert.
//!
//! The bead's acceptance criterion is "draft upsert is idempotent under rapid
//! autosave".

use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    Attachment, Draft, DraftId, DraftKind, DraftState, EmailAddress, Message, MessageBody,
    MessageId, Operation, OperationTarget, ThreadId,
};
use postio_storage::bind;
use postio_storage::repository::{
    CancelSendOutcome, DraftRepository, MessageRepository, OperationQueueRepository,
};
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

#[tokio::test]
async fn a_draft_round_trips_with_its_recipients_and_attachments() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::Reply;
    draft.bcc = vec![EmailAddress::new(None::<String>, "archive@example.com")];
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 1_024);
    attachment.filename = Some("revision-c.pdf".to_owned());
    draft.attachments = vec![attachment];

    let id = drafts.save(&mut draft).await.expect("save");

    assert!(id.is_assigned());
    assert_eq!(draft.id, id);
    assert!(draft.attachments[0].id.is_assigned());

    let stored = drafts.get(id).await.expect("get").expect("the draft");
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

#[tokio::test]
async fn the_body_of_a_draft_is_stored_inline_and_not_in_the_blob_store() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.body.html = Some("<p>Half a sentence</p>".to_owned());
    let id = drafts.save(&mut draft).await.expect("save");

    let (text, html): (Option<String>, Option<String>) = postio_storage::sql::one(
        &connection,
        "SELECT body_text, body_html FROM drafts WHERE id = ?1",
        bind![id.get()],
        |row| {
            Ok((
                postio_storage::sql::RowExt::col(row, 0)?,
                postio_storage::sql::RowExt::col(row, 1)?,
            ))
        },
    )
    .await
    .expect("read the raw row");

    assert_eq!(text.as_deref(), Some("Half a sentence"));
    assert_eq!(html.as_deref(), Some("<p>Half a sentence</p>"));
}

#[tokio::test]
async fn reading_a_draft_that_is_not_there_is_none() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let drafts = DraftRepository::new(&connection);

    assert!(drafts.get(DraftId::new(404)).await.expect("get").is_none());
    assert!(!drafts.delete(DraftId::new(404)).await.expect("delete"));
}

// ---------------------------------------------------------------------------
// Acceptance: autosave is idempotent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn saving_the_same_draft_repeatedly_writes_one_row() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let id = drafts.save(&mut draft).await.expect("first save");

    // The composer autosaves on every keystroke.
    for keystroke in 1..=50 {
        draft.subject = format!("Tide gate interlock{}", "!".repeat(keystroke));
        draft.updated_at = at(keystroke as i64);
        let same = drafts.save(&mut draft).await.expect("autosave");
        assert_eq!(same, id, "autosave never starts a second draft");
    }

    for (table, expected) in [("drafts", 1), ("recipients", 2), ("attachments", 0)] {
        let count: i64 = postio_storage::sql::one(
            &connection,
            &format!("SELECT count(*) FROM {table}"),
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("count");
        assert_eq!(count, expected, "{table} must not accumulate");
    }

    let stored = drafts.get(id).await.expect("get").expect("the draft");
    assert_eq!(stored.subject, draft.subject);
    assert_eq!(stored.updated_at, at(50), "but the timestamp moves");
    assert_eq!(stored.created_at, at(0), "and the start does not");
}

#[tokio::test]
async fn autosave_keeps_attachment_identity_stable() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.attachments = vec![Attachment::new(MessageId::UNASSIGNED, "image/png", 64)];
    let id = drafts.save(&mut draft).await.expect("save");
    let attachment_id = draft.attachments[0].id;

    draft.body.text = Some("More text".to_owned());
    drafts.save(&mut draft).await.expect("autosave");

    let stored = drafts.get(id).await.expect("get").expect("the draft");
    assert_eq!(stored.attachments.len(), 1);
    assert_eq!(
        stored.attachments[0].id, attachment_id,
        "an attachment the user added keeps its id across autosaves"
    );
}

#[tokio::test]
async fn removing_a_recipient_removes_the_row() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let id = drafts.save(&mut draft).await.expect("save");

    draft.cc.clear();
    drafts.save(&mut draft).await.expect("autosave");

    let stored = drafts.get(id).await.expect("get").expect("the draft");
    assert!(stored.cc.is_empty());
    assert_eq!(stored.to.len(), 1);
}

// ---------------------------------------------------------------------------
// The send queue and the composer's other reads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drafts_list_most_recently_edited_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut older = a_draft(account.id);
    older.updated_at = at(1);
    let older_id = drafts.save(&mut older).await.expect("save");
    let mut newer = a_draft(account.id);
    newer.updated_at = at(2);
    let newer_id = drafts.save(&mut newer).await.expect("save");

    let listed: Vec<DraftId> = drafts
        .list_for_account(account.id)
        .await
        .expect("list")
        .iter()
        .map(|draft| draft.id)
        .collect();

    assert_eq!(listed, [newer_id, older_id]);
}

#[tokio::test]
async fn the_send_queue_reads_drafts_by_state_in_the_order_they_were_queued() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut first = a_draft(account.id);
    first.updated_at = at(1);
    let first_id = drafts.save(&mut first).await.expect("save");
    let mut second = a_draft(account.id);
    second.updated_at = at(2);
    let second_id = drafts.save(&mut second).await.expect("save");
    let mut editing = a_draft(account.id);
    drafts.save(&mut editing).await.expect("save");

    drafts
        .set_state(first_id, DraftState::Queued)
        .await
        .expect("queue");
    drafts
        .set_state(second_id, DraftState::Queued)
        .await
        .expect("queue");

    let queued: Vec<DraftId> = drafts
        .by_state(DraftState::Queued)
        .await
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
            .await
            .expect("get")
            .expect("the draft")
            .is_sendable(),
        "a queued draft is no longer the composer's to send again"
    );
}

#[tokio::test]
async fn a_reply_draft_remembers_the_message_and_thread_it_belongs_to() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let drafts = DraftRepository::new(&connection);

    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1)",
            [account.id.get()],
        )
        .await
        .expect("a thread");
    let mut parent = Message::new(account.id, inbox, at(0));
    parent.subject = Some("Tide gate interlock".to_owned());
    MessageRepository::new(&connection)
        .create(&mut parent)
        .await
        .expect("create");

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::ReplyAll;
    draft.in_reply_to = Some(parent.id);
    draft.thread_id = Some(ThreadId::new(1));
    let id = drafts.save(&mut draft).await.expect("save");

    let stored = drafts.get(id).await.expect("get").expect("the draft");
    assert_eq!(stored.in_reply_to, Some(parent.id));
    assert_eq!(stored.thread_id, Some(ThreadId::new(1)));
}

#[tokio::test]
async fn a_draft_survives_the_message_it_replies_to_being_expunged() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut parent = Message::new(account.id, inbox, at(0));
    MessageRepository::new(&connection)
        .create(&mut parent)
        .await
        .expect("create");
    let mut draft = a_draft(account.id);
    draft.in_reply_to = Some(parent.id);
    let id = drafts.save(&mut draft).await.expect("save");

    MessageRepository::new(&connection)
        .delete(&[parent.id])
        .await
        .expect("expunge");

    let stored = drafts
        .get(id)
        .await
        .expect("get")
        .expect("the draft is still there");
    assert_eq!(
        stored.in_reply_to, None,
        "losing the parent must never lose what the user typed"
    );
}

#[tokio::test]
async fn a_forward_keeps_the_message_it_was_made_from_until_that_is_expunged() {
    // #1686: a forward's carried attachment may have no bytes yet, and the
    // original is where they are fetched from at send time.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut source = Message::new(account.id, inbox, at(0));
    MessageRepository::new(&connection)
        .create(&mut source)
        .await
        .expect("create");
    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::Forward;
    draft.forwarded_from = Some(source.id);
    let id = drafts.save(&mut draft).await.expect("save");

    let stored = drafts.get(id).await.expect("get").expect("the draft");
    assert_eq!(stored.forwarded_from, Some(source.id));
    assert_eq!(stored.in_reply_to, None, "a forward threads nowhere");

    MessageRepository::new(&connection)
        .delete(&[source.id])
        .await
        .expect("expunge");
    let stored = drafts.get(id).await.expect("get").expect("still there");
    assert_eq!(stored.forwarded_from, None);
}

#[tokio::test]
async fn a_carried_attachment_takes_its_bytes_once_they_are_fetched() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _inbox) = test_support::account_with_inbox(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let mut carried = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 12);
    carried.filename = Some("statement.pdf".to_owned());
    carried.part_id = Some("2".to_owned());
    draft.attachments = vec![carried];
    let id = drafts.save(&mut draft).await.expect("save");
    let attachment = draft.attachments[0].id;
    let blob = postio_model::BlobId::new("d".repeat(64));

    assert!(
        drafts
            .set_attachment_blob(id, attachment, &blob)
            .await
            .expect("set")
    );

    let stored = drafts.get(id).await.expect("get").expect("the draft");
    assert_eq!(stored.attachments[0].blob_id.as_ref(), Some(&blob));
    assert_eq!(
        stored.attachments[0].id, attachment,
        "the composer's id for the row still names it"
    );
    assert!(
        !drafts
            .set_attachment_blob(DraftId::new(id.get() + 1), attachment, &blob)
            .await
            .expect("set"),
        "another draft's attachment is not this one's to fill"
    );
}

#[tokio::test]
async fn deleting_a_draft_takes_its_recipients_and_attachments() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.attachments = vec![Attachment::new(MessageId::UNASSIGNED, "image/png", 8)];
    let id = drafts.save(&mut draft).await.expect("save");

    assert!(drafts.delete(id).await.expect("delete"));

    for table in ["drafts", "recipients", "attachments"] {
        let count: i64 = postio_storage::sql::one(
            &connection,
            &format!("SELECT count(*) FROM {table}"),
            (),
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("count");
        assert_eq!(count, 0, "{table}");
    }
}

#[tokio::test]
async fn enumerations_are_stored_with_the_spelling_the_model_documents() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::Forward;
    let id = drafts.save(&mut draft).await.expect("save");
    drafts
        .set_state(id, DraftState::Failed)
        .await
        .expect("fail it");

    let (kind, state): (String, String) = postio_storage::sql::one(
        &connection,
        "SELECT kind, state FROM drafts WHERE id = ?1",
        bind![id.get()],
        |row| {
            Ok((
                postio_storage::sql::RowExt::col(row, 0)?,
                postio_storage::sql::RowExt::col(row, 1)?,
            ))
        },
    )
    .await
    .expect("read the raw row");

    assert_eq!(kind, DraftKind::Forward.as_str());
    assert_eq!(state, DraftState::Failed.as_str());
    assert!(
        drafts
            .get(id)
            .await
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
async fn account_with_drafts(
    connection: &Connection,
) -> (postio_model::Account, postio_model::MailboxId) {
    let account = test_support::account(connection).await;
    let drafts = test_support::mailbox(connection, &account, "Drafts").await;
    assert_eq!(
        drafts.role,
        postio_model::MailboxRole::Drafts,
        "the fixture depends on the role being derived from the path"
    );
    (account, drafts.id)
}

#[tokio::test]
async fn saving_a_draft_queues_it_for_the_server_in_the_same_write() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let queued = drafts
        .save_and_sync(&mut draft, at(0))
        .await
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
        drafts.get(draft.id).await.expect("get").is_some(),
        "the local row is written whether or not the server ever hears about it"
    );
}

#[tokio::test]
async fn a_draft_with_nowhere_to_go_is_still_saved_locally() {
    // No Drafts mailbox: the account has not been synced far enough to know
    // one exists. Local-first means the draft is kept anyway, and the next
    // save after the folder turns up is what files it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let queued = drafts.save_and_sync(&mut draft, at(0)).await.expect("save");

    assert!(queued.is_none(), "there is no folder to file it in");
    assert!(drafts.get(draft.id).await.expect("get").is_some());
}

#[tokio::test]
async fn discarding_a_draft_removes_it_locally_and_queues_the_server_copy() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.server.uid = Some(postio_model::Uid::new(41));
    draft.server.uid_validity = Some(postio_model::UidValidity::new(9));
    draft.server.remote_id = Some(postio_model::RemoteId::new("9:41"));
    drafts.save(&mut draft).await.expect("save");

    let queued = drafts
        .discard(draft.id, at(1))
        .await
        .expect("discard")
        .expect("a queue row");

    assert_eq!(
        queued.operation,
        Operation::DiscardDraft {
            mailbox: drafts_mailbox,
            remote_id: postio_model::RemoteId::new("9:41"),
        },
        "the operation carries the copy to remove, because the row that knew \
         it is about to be gone"
    );
    assert!(
        drafts.get(draft.id).await.expect("get").is_none(),
        "the draft is gone here the moment the user says so"
    );
}

#[tokio::test]
async fn discarding_a_draft_the_server_never_saw_queues_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");

    assert!(
        drafts
            .discard(draft.id, at(1))
            .await
            .expect("discard")
            .is_none(),
        "nothing was uploaded, so there is nothing to remove"
    );
    assert!(drafts.get(draft.id).await.expect("get").is_none());
    let pending = OperationQueueRepository::new(&connection)
        .pending(account.id, at(2))
        .await
        .expect("pending");
    assert!(pending.is_empty(), "and no round trip is spent saying so");
}

#[tokio::test]
async fn discarding_a_draft_that_is_already_gone_is_not_an_error() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    assert!(
        drafts
            .discard(DraftId::new(404), at(1))
            .await
            .expect("discard")
            .is_none(),
        "a retried discard is the expected case, not a failure"
    );
}

#[tokio::test]
async fn queueing_a_draft_for_sending_marks_it_and_enqueues_the_operation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");

    let queued = drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert_eq!(queued.target, OperationTarget::Draft(draft.id));
    assert_eq!(
        drafts
            .get(draft.id)
            .await
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

#[tokio::test]
async fn scheduling_a_send_marks_it_queued_but_holds_it_until_the_chosen_time() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");

    let send_at = at(120);
    let queued = drafts
        .queue_send_at(&mut draft, at(1), send_at)
        .await
        .expect("schedule the send");

    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert_eq!(queued.target, OperationTarget::Draft(draft.id));
    assert_eq!(
        queued.next_attempt_at,
        Some(send_at),
        "the drainer must not touch this before the chosen time"
    );
    assert_eq!(queued.attempts, 0, "nothing has been attempted yet");
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("the draft is still here")
            .state,
        DraftState::Queued,
        "queued the same way an immediate send is — the composer has let go \
         of it either way"
    );

    let queue = OperationQueueRepository::new(&connection);
    let too_early = queue.pending(account.id, at(60)).await.expect("pending");
    assert!(
        too_early.is_empty(),
        "the scheduled send must not drain before its time"
    );
    let due = queue.pending(account.id, send_at).await.expect("pending");
    assert_eq!(
        due.into_iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![queued.id],
        "and must drain once its time arrives"
    );
}

#[tokio::test]
async fn queueing_a_send_writes_the_draft_that_was_never_saved() {
    // Ctrl+Enter can beat the debounced autosave: a draft typed and sent
    // inside the quiet period has no row and no id yet. The enqueue names the
    // draft by id, so there is no send to queue until there is one.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    assert!(!draft.id.is_assigned(), "nothing has saved it yet");

    let queued = drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    assert!(draft.id.is_assigned());
    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert!(drafts.get(draft.id).await.expect("get").is_some());
}

#[tokio::test]
async fn a_queued_send_does_not_need_a_drafts_mailbox() {
    // Unlike `save_and_sync`, which has nowhere to file the draft until the
    // first sync finds the folder, a send names no mailbox at all: SMTP is a
    // different conversation from IMAP, and the Sent copy is resolved when the
    // operation drains rather than now.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    let queued = drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    assert_eq!(queued.operation, Operation::Send { draft: draft.id });
    assert!(queued.mailbox_id.is_none());
}

#[tokio::test]
async fn cancelling_a_queued_send_takes_the_operation_off_the_queue_and_reopens_the_draft() {
    // #433: a queued draft could still be opened and edited from the Drafts
    // folder, and the edits landed or did not depending purely on timing
    // against the drainer. `OperationQueueRepository::delete`'s own doc
    // already names the fix -- "an operation that has not drained yet can
    // simply be taken off the queue" -- this is that, wired to the draft
    // that owns it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    let queued = drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    let outcome = drafts
        .cancel_send(draft.id, at(2))
        .await
        .expect("cancel the send");

    assert_eq!(outcome, CancelSendOutcome::Cancelled);
    let reopened = drafts
        .get(draft.id)
        .await
        .expect("get")
        .expect("the draft is still here");
    assert_eq!(
        reopened.state,
        DraftState::Editing,
        "cancelling a send must hand the draft back for editing"
    );

    let queue = OperationQueueRepository::new(&connection);
    assert!(
        queue.get(queued.id).await.expect("get").is_none(),
        "the pending Send operation must be gone, or the drainer could still \
         pick it up and send whatever was in the row when it drains"
    );
}

#[tokio::test]
async fn cancelling_the_send_of_a_draft_that_is_not_queued_does_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");

    let outcome = drafts
        .cancel_send(draft.id, at(2))
        .await
        .expect("cancel the send");

    assert_eq!(outcome, CancelSendOutcome::NotQueued);
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("still here")
            .state,
        DraftState::Editing,
        "a draft that was never queued must not change state"
    );
}

#[tokio::test]
async fn cancelling_a_send_that_is_already_in_flight_is_too_late() {
    // The drainer marks the operation in_flight the moment it starts talking
    // to the submission server. From then on the row leaving the queue
    // cannot take back a message that may already be on the wire, so this
    // must refuse rather than pretend the send never happened.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    let queued = drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .mark_in_flight(queued.id, at(2))
        .await
        .expect("mark in flight");

    let outcome = drafts
        .cancel_send(draft.id, at(3))
        .await
        .expect("cancel the send");

    assert_eq!(outcome, CancelSendOutcome::AlreadyInFlight);
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("still here")
            .state,
        DraftState::Queued,
        "an in-flight send must not be disturbed"
    );
    assert!(
        queue.get(queued.id).await.expect("get").is_some(),
        "the operation must stay on the queue"
    );
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
    message.server.remote_id = Some(postio_model::RemoteId::new(format!("{validity}:{uid}")));
    message
}

/// A draft whose server copy is `uid` under `validity`.
async fn uploaded(
    connection: &Connection,
    account: postio_model::AccountId,
    uid: u32,
    validity: u32,
) -> DraftId {
    let mut draft = a_draft(account);
    let drafts = DraftRepository::new(connection);
    let id = drafts.save(&mut draft).await.expect("save the draft");
    drafts
        .set_server_copy(
            id,
            Some(&postio_storage::repository::ServerCopyLocation {
                remote_id: postio_model::RemoteId::new(format!("{validity}:{uid}")),
                uid: postio_model::Uid::new(uid),
                uid_validity: postio_model::UidValidity::new(validity),
            }),
        )
        .await
        .expect("record where the append landed");
    id
}

async fn rows_in(connection: &Connection, mailbox: postio_model::MailboxId) -> u32 {
    MessageRepository::new(connection)
        .count_set(&postio_storage::repository::MessageSet::in_mailbox(mailbox))
        .await
        .expect("a count")
}

#[tokio::test]
async fn a_draft_this_client_uploaded_does_not_come_back_as_a_message() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts) = account_with_drafts(&connection).await;
    uploaded(&connection, account.id, 7, 1).await;

    let mut batch = vec![fetched(account.id, drafts, 7, 1)];
    let report = MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over Drafts");

    assert_eq!(report.inserted, 0);
    assert_eq!(report.own_drafts, 1);
    assert!(
        batch.is_empty(),
        "the batch is what the caller goes on to thread and record contacts \
         from, so a skipped message has to leave it"
    );
    assert_eq!(
        rows_in(&connection, drafts).await,
        1,
        "the one row is the one `save` wrote for the folder to list (#166); \
         the copy that came back down added nothing"
    );
}

#[tokio::test]
async fn a_draft_written_by_another_client_is_an_ordinary_message() {
    // The reason the folder syncs at all. This one has no local draft row, so
    // there is nothing for it to be a second copy of.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts) = account_with_drafts(&connection).await;
    uploaded(&connection, account.id, 7, 1).await;

    let mut batch = vec![
        fetched(account.id, drafts, 7, 1),
        fetched(account.id, drafts, 8, 1),
    ];
    let report = MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over Drafts");

    assert_eq!(report.inserted, 1);
    assert_eq!(report.own_drafts, 1);
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].server.uid.map(postio_model::Uid::get), Some(8));
    assert_eq!(
        rows_in(&connection, drafts).await,
        2,
        "the draft's own row, and the other client's draft beside it"
    );
}

#[tokio::test]
async fn a_uid_that_matches_a_draft_in_a_different_folder_is_an_ordinary_message() {
    // UIDs are per-mailbox, so message 7 in INBOX has nothing to do with the
    // draft that is message 7 in Drafts. Matching on the number alone would
    // hide a piece of the user's mail.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _drafts) = account_with_drafts(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX")
        .await
        .id;
    uploaded(&connection, account.id, 7, 1).await;

    let mut batch = vec![fetched(account.id, inbox, 7, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over INBOX");

    assert_eq!(rows_in(&connection, inbox).await, 1);
}

#[tokio::test]
async fn a_draft_whose_append_was_never_located_hides_nothing() {
    // No `UIDPLUS`, so `save` recorded that it does not know where the copy
    // landed and flagged the folder for a resync instead. There is nothing to
    // match on, and guessing which message in Drafts is ours is exactly what
    // `postio-sync`'s draft module refuses to do.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts) = account_with_drafts(&connection).await;
    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save the draft");

    let mut batch = vec![fetched(account.id, drafts, 7, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over Drafts");

    assert_eq!(
        rows_in(&connection, drafts).await,
        2,
        "the draft's own row, and the copy of it that came back down — \
         nothing links the two, which is what `postio-sync::drafts` flags the \
         folder for a resync over rather than guessing about"
    );
}

#[tokio::test]
async fn a_draft_recorded_under_an_older_generation_hides_nothing() {
    // A renumbered mailbox makes the old UID name some other message, which
    // is the same reason `discard` carries its `UIDVALIDITY`.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts) = account_with_drafts(&connection).await;
    uploaded(&connection, account.id, 7, 1).await;

    let mut batch = vec![fetched(account.id, drafts, 7, 2)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over a renumbered Drafts");

    assert_eq!(
        rows_in(&connection, drafts).await,
        2,
        "the draft's own row, and the message that is number 7 under the new \
         generation — which is not the same message at all"
    );
}

#[tokio::test]
async fn a_message_row_that_beat_the_draft_to_its_uid_is_taken_back_out() {
    // The race the skip alone does not close: a sync pass fetched the
    // appended copy before `set_server_copy` had recorded where it landed, so
    // the row was already there when the draft learned its own UID. Every
    // later pass would then find the row and update it, and the duplicate
    // would be permanent. Claiming the copy is therefore also what removes it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts) = account_with_drafts(&connection).await;
    let elsewhere = test_support::mailbox(&connection, &account, "INBOX")
        .await
        .id;
    let mut first = vec![
        fetched(account.id, drafts, 7, 1),
        fetched(account.id, elsewhere, 7, 1),
    ];
    MessageRepository::new(&connection)
        .upsert_batch(&mut first)
        .await
        .expect("a pass that ran before the draft was linked");
    assert_eq!(
        rows_in(&connection, drafts).await,
        1,
        "the duplicate this repairs"
    );

    uploaded(&connection, account.id, 7, 1).await;

    assert_eq!(
        rows_in(&connection, drafts).await,
        1,
        "the stray row goes and the draft's own row is what is left, rather \
         than the two of them sitting side by side"
    );
    assert_eq!(
        rows_in(&connection, elsewhere).await,
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
async fn folder(connection: &Connection, mailbox: postio_model::MailboxId) -> Vec<String> {
    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(mailbox),
        limit: 50,
        after: None,
    };
    MessageRepository::new(connection)
        .page(&query)
        .await
        .expect("a page")
        .into_iter()
        .map(|row| row.subject.unwrap_or_default())
        .collect()
}

/// The count the sidebar draws under "Drafts".
async fn badge(connection: &Connection, mailbox: postio_model::MailboxId) -> u32 {
    postio_storage::repository::MailboxRepository::new(connection)
        .counts(mailbox)
        .await
        .expect("a read")
        .expect("the mailbox")
        .total
}

#[tokio::test]
async fn saving_a_draft_puts_it_in_the_drafts_folder_at_once() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;

    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save the draft");

    assert_eq!(
        folder(&connection, drafts_mailbox).await,
        vec!["Tide gate interlock".to_owned()],
        "no server round trip stands between typing and this"
    );
    assert_eq!(badge(&connection, drafts_mailbox).await, 1);
}

#[tokio::test]
async fn the_row_a_draft_owns_is_marked_as_a_draft_and_as_read() {
    // The list already draws a draft mark and says "Draft" in the accessible
    // label off `MessageListRow::draft`; unread is for mail that arrived.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;

    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("save the draft");

    let query = postio_storage::repository::ListQuery {
        scope: postio_storage::repository::ListScope::Mailbox(drafts_mailbox),
        limit: 50,
        after: None,
    };
    let rows = MessageRepository::new(&connection)
        .page(&query)
        .await
        .expect("a page");
    assert!(rows[0].is_draft(), "the row the folder shows is a draft");
    assert!(rows[0].seen, "your own draft is not unread mail");
    assert_eq!(badge(&connection, drafts_mailbox).await, 1);
}

#[tokio::test]
async fn autosave_keeps_one_row_and_keeps_it_current() {
    // The autosave rule this repository already holds for the draft row, held
    // for its list row too: a keystroke is not a new draft.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    draft.subject = "Tide gate interlock, revised".to_owned();
    draft.updated_at = at(5);
    drafts.save(&mut draft).await.expect("save again");

    assert_eq!(
        folder(&connection, drafts_mailbox).await,
        vec!["Tide gate interlock, revised".to_owned()]
    );
    assert_eq!(badge(&connection, drafts_mailbox).await, 1);
}

#[tokio::test]
async fn discarding_a_draft_takes_its_row_out_of_the_folder() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts.discard(draft.id, at(5)).await.expect("discard");

    assert!(folder(&connection, drafts_mailbox).await.is_empty());
    assert_eq!(badge(&connection, drafts_mailbox).await, 0);
}

#[tokio::test]
async fn sending_a_draft_takes_its_row_out_of_the_folder() {
    // `postio-sync::send` finishes by deleting the draft, which is the single
    // exit both it and discard go through.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    assert!(drafts.delete(draft.id).await.expect("delete"));

    assert!(folder(&connection, drafts_mailbox).await.is_empty());
    assert_eq!(badge(&connection, drafts_mailbox).await, 0);
}

#[tokio::test]
async fn an_account_with_no_drafts_folder_yet_still_saves_the_draft() {
    // The ordinary state of an account that has not finished its first sync.
    // The draft is durable regardless; it simply has nowhere to be listed.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;

    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("a draft is durable before the folder exists");

    assert!(draft.id.is_assigned());
}

#[tokio::test]
async fn the_row_a_draft_owns_is_the_one_its_server_copy_attaches_to() {
    // The two halves have to meet. The append lands, `set_server_copy` records
    // where — and the row the folder is already showing becomes the row that
    // names that copy, rather than a second one appearing beside it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .set_server_copy(
            draft.id,
            Some(&postio_storage::repository::ServerCopyLocation {
                remote_id: postio_model::RemoteId::new("1:7"),
                uid: postio_model::Uid::new(7),
                uid_validity: postio_model::UidValidity::new(1),
            }),
        )
        .await
        .expect("record where the append landed");

    assert_eq!(folder(&connection, drafts_mailbox).await.len(), 1);

    // And the sync pass that fetches the copy back still adds nothing: #51.
    let mut batch = vec![fetched(account.id, drafts_mailbox, 7, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over Drafts");

    assert_eq!(
        folder(&connection, drafts_mailbox).await,
        vec!["Tide gate interlock".to_owned()],
        "one draft, one row, whichever half wrote it"
    );
    assert_eq!(badge(&connection, drafts_mailbox).await, 1);
}

#[tokio::test]
async fn a_drafts_row_leads_back_to_the_draft_it_is_listing() {
    // The link the other way. The message list hands back a `MessageId`, and
    // activating a draft's row has to reach the buffer the composer edits —
    // opening the reader on it instead is the dead end #166 is about.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
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
        .await
        .expect("three drafts written elsewhere");

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    let listed = MessageRepository::new(&connection)
        .page(&postio_storage::repository::ListQuery {
            scope: postio_storage::repository::ListScope::Mailbox(drafts_mailbox),
            limit: 50,
            after: None,
        })
        .await
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
        .await
        .expect("a read")
        .expect("the row is a draft's, so there is one");

    assert_eq!(found.id, draft.id);
    assert_eq!(found.subject, draft.subject);
    assert_eq!(
        found.to, draft.to,
        "the whole draft, recipients and all — the composer opens on this"
    );
}

#[tokio::test]
async fn a_message_that_is_not_a_drafts_row_leads_nowhere() {
    // Another client's draft, which has no local buffer to open. What happens
    // then is `postio-app`'s decision; what is certain here is that there is
    // nothing to find — including when its row number happens to be a draft's
    // id, which is the coincidence a link keyed on the wrong column survives.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let mut draft = a_draft(account.id);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("a draft, so there is an id to collide with");

    let mut batch = vec![fetched(account.id, drafts_mailbox, 9, 1)];
    MessageRepository::new(&connection)
        .upsert_batch(&mut batch)
        .await
        .expect("a sync pass over Drafts");
    let foreign = batch[0].id;

    assert!(
        DraftRepository::new(&connection)
            .by_message(foreign)
            .await
            .expect("a read")
            .is_none()
    );
    // And the one that *is* a draft's still leads to it, so the assertion
    // above is about the link rather than about `by_message` finding nothing.
    let mine = DraftRepository::new(&connection)
        .get(draft.id)
        .await
        .expect("a read")
        .expect("the draft");
    assert_eq!(mine.id, draft.id);
}

// ---------------------------------------------------------------------------
// The reserved Message-ID (ADR 0021)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn queueing_a_send_reserves_the_message_id_it_will_go_out_under() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    assert_eq!(
        draft.rfc_message_id, None,
        "an editing draft has reserved nothing yet"
    );

    drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    let reserved = draft
        .rfc_message_id
        .clone()
        .expect("the caller's copy carries the reservation");
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("the draft")
            .rfc_message_id,
        Some(reserved.clone()),
        "and so does the row the drainer will read"
    );
    assert!(
        reserved.without_brackets().contains('@'),
        "a Message-ID is addr-spec shaped: {reserved}"
    );
}

#[tokio::test]
async fn a_scheduled_send_reserves_one_the_same_way() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .queue_send_at(&mut draft, at(1), at(120))
        .await
        .expect("schedule the send");

    assert!(
        draft.rfc_message_id.is_some(),
        "a send held until Tuesday is still one send attempt series"
    );
}

/// The point of the whole reservation: it has to outlive every write that
/// happens between queueing and draining, because a drain rebuilds the
/// message from the row.
#[tokio::test]
async fn the_reservation_survives_the_saves_that_happen_after_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("reserved");

    // A save from whoever still holds the draft -- the drainer writing the
    // server copy's uid, say -- must not lose it.
    let mut reloaded = drafts.get(draft.id).await.expect("get").expect("the draft");
    reloaded.updated_at = at(2);
    drafts.save(&mut reloaded).await.expect("save again");

    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("the draft")
            .rfc_message_id,
        Some(reserved),
    );
}

/// ADR 0021: the id belongs to one attempt series at one piece of text. A
/// draft that is editable again is a different message, and reusing the id
/// would make a receiver that deduplicates drop the *corrected* version in
/// favour of the one that may already have arrived.
#[tokio::test]
async fn cancelling_a_send_gives_the_reservation_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");
    assert!(draft.rfc_message_id.is_some());

    assert_eq!(
        drafts.cancel_send(draft.id, at(2)).await.expect("cancel"),
        CancelSendOutcome::Cancelled,
    );

    let back = drafts.get(draft.id).await.expect("get").expect("the draft");
    assert_eq!(back.state, DraftState::Editing);
    assert_eq!(
        back.rfc_message_id, None,
        "an editable draft holds no reservation: the next send is a new \
         attempt series, at text that may no longer be the same text"
    );
}

/// A row that says `editing` and still carries a reservation cannot be
/// written at all, whichever way the caller asks for it — the column is
/// derived from the state rather than trusted from the struct.
#[tokio::test]
async fn an_editing_draft_cannot_be_saved_holding_a_reservation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    draft.rfc_message_id = Some(postio_model::RfcMessageId::new("stale@example.com"));
    drafts.save(&mut draft).await.expect("save");

    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("the draft")
            .rfc_message_id,
        None,
    );
}

/// `Sending` and "there was never anything queued" are opposite facts and
/// must not share an answer: one means nothing happened, the other means a
/// submission may already be on the wire.
#[tokio::test]
async fn cancelling_a_send_already_being_submitted_says_it_is_in_flight() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");
    drafts
        .set_state(draft.id, DraftState::Sending)
        .await
        .expect("the drainer opens the transaction");

    assert_eq!(
        drafts.cancel_send(draft.id, at(2)).await.expect("cancel"),
        CancelSendOutcome::AlreadyInFlight,
        "telling the user a message was recalled while it is on the wire is \
         worse than refusing"
    );
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("the draft")
            .state,
        DraftState::Sending,
        "and it stays where it was",
    );
}

#[tokio::test]
async fn moving_a_draft_back_to_editing_gives_the_reservation_back_too() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    drafts
        .set_state(draft.id, DraftState::Editing)
        .await
        .expect("set_state");

    let back = drafts.get(draft.id).await.expect("get").expect("the draft");
    assert_eq!(
        back.rfc_message_id, None,
        "the invariant holds on every write path or it is not an invariant: \
         `save` is not the only way a draft becomes editable again"
    );
    assert_eq!(
        drafts
            .get(draft.id)
            .await
            .expect("get")
            .expect("the draft")
            .state,
        DraftState::Editing,
    );
}

#[tokio::test]
async fn set_state_leaves_the_reservation_alone_for_every_other_state() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");
    let reserved = draft.rfc_message_id.clone().expect("reserved");

    for state in [DraftState::Sending, DraftState::Sent, DraftState::Failed] {
        drafts.set_state(draft.id, state).await.expect("set_state");
        assert_eq!(
            drafts
                .get(draft.id)
                .await
                .expect("get")
                .expect("the draft")
                .rfc_message_id,
            Some(reserved.clone()),
            "{state:?} is still the same attempt series",
        );
    }
}

// ---------------------------------------------------------------------------
// What it costs (Constitution V, spec 002 T055)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn saving_and_loading_a_draft_costs_a_fixed_number_of_statements() {
    // Autosave runs on a debounce while somebody types, so this is a write
    // path that repeats for the life of every draft -- and loading one is
    // what stands between pressing Return on a Drafts row and seeing the
    // composer. Both are budgeted as *counts* rather than timings, because a
    // shared runner cannot defend 16 ms and these numbers are the same on any
    // machine (`bench.yml` deliberately times nothing).
    //
    // What the numbers are guarding is shape, not speed: an N+1 over
    // recipients or attachments does not show up as a slow test on a draft
    // with two of each, it shows up here as a count that grew with the data.
    // That is why the second half adds parts and asserts the cost did *not*
    // move with them.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);

    postio_storage::test_support::counting::install(&connection);

    let mut small = a_draft(account.id);
    let saving = postio_storage::test_support::counting::counted_async(|| async {
        drafts.save(&mut small).await.expect("save");
    })
    .await;
    let loading = postio_storage::test_support::counting::counted_async(|| async {
        drafts
            .get(small.id)
            .await
            .expect("get")
            .expect("still here");
    })
    .await;

    // Loading is pinned exactly, because 3 is a number with a meaning -- the
    // draft row, its recipients, its attachments -- and any fourth statement
    // is a question worth answering rather than drift to absorb.
    assert_eq!(
        loading.statements, 3,
        "loading a draft is the row, its recipients and its parts: {loading:?}"
    );
    // Saving is a ceiling, not an equality. It writes several tables inside a
    // transaction and the exact count moves for reasons that teach nobody
    // anything; what must not happen is that it doubles unnoticed, because
    // autosave runs this on a debounce for the life of every draft.
    assert!(
        saving.statements <= 32,
        "saving an almost-empty draft took {} statements: {saving:?}",
        saving.statements
    );

    // ── And the cost does not grow with the draft's contents ─────────────
    let mut large = a_draft(account.id);
    large.to = (0..8)
        .map(|n| EmailAddress::new(None::<String>, format!("to{n}@example.net")))
        .collect();
    large.cc = (0..8)
        .map(|n| EmailAddress::new(None::<String>, format!("cc{n}@example.org")))
        .collect();
    large.attachments = (0..8)
        .map(|n| {
            let mut part = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 1_024);
            part.filename = Some(format!("report-{n}.pdf"));
            part
        })
        .collect();

    let saving_large = postio_storage::test_support::counting::counted_async(|| async {
        drafts.save(&mut large).await.expect("save");
    })
    .await;
    let loading_large = postio_storage::test_support::counting::counted_async(|| async {
        drafts
            .get(large.id)
            .await
            .expect("get")
            .expect("still here");
    })
    .await;

    // Writing more rows is more statements and that is honest work. Reading
    // is where an N+1 hides, because one query per attachment looks exactly
    // like one query for all of them until the draft has eight.
    assert_eq!(
        loading_large.statements, loading.statements,
        "loading a draft with sixteen recipients and eight attachments cost \
         {} statements against {} for an almost empty one, which is a query \
         per part rather than a query for all of them",
        loading_large.statements, loading.statements
    );
    // Writing more rows is more statements and that is honest work; what is
    // budgeted is the rate. Measured at about four per row -- a delete, an
    // insert and the bookkeeping around them -- so six per row leaves room
    // for an incidental change and none for a doubling.
    let extra_rows = (16 - 1) + 8;
    assert!(
        saving_large.statements <= saving.statements + extra_rows * 6,
        "saving grew faster than six statements per row written: \
         {saving_large:?} against {saving:?} for {extra_rows} more rows"
    );
}

#[tokio::test]
async fn a_failed_send_leaves_the_draft_editable_and_the_reason_where_it_can_be_found() {
    // FR-066, in the two thirds that are built. A send that fails must leave
    // something a person can act on: the draft still theirs to edit, still in
    // the Drafts folder, and the reason recorded rather than discarded with
    // the attempt.
    //
    // The third part -- that the reason is *named* to the user -- is not
    // asserted here because nothing shows it (#1487). It is computed, written
    // to `last_error`, and carried all the way up the engine's drain report,
    // and then no surface reads it.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, mailbox) = account_with_drafts(&connection).await;
    let drafts = DraftRepository::new(&connection);
    let queue = OperationQueueRepository::new(&connection);

    // A reply to a real message, so the threading assertion below has
    // something to lose rather than comparing two `None`s.
    let mut parent = Message::new(account.id, mailbox, at(0));
    parent.subject = Some("Tide gate interlock".to_owned());
    MessageRepository::new(&connection)
        .create(&mut parent)
        .await
        .expect("create the parent");

    let mut draft = a_draft(account.id);
    draft.kind = DraftKind::Reply;
    draft.in_reply_to = Some(parent.id);
    drafts.save(&mut draft).await.expect("save");
    let queued = drafts
        .queue_send(&mut draft, at(1))
        .await
        .expect("queue the send");

    queue
        .mark_failed(queued.id, at(2), "550 mailbox unavailable")
        .await
        .expect("mark failed");
    drafts
        .set_state(draft.id, DraftState::Failed)
        .await
        .expect("the draft learns the send failed");

    // ── Still editable ───────────────────────────────────────────────────
    let after = drafts
        .get(draft.id)
        .await
        .expect("get")
        .expect("still here");
    assert_eq!(after.state, DraftState::Failed);
    assert!(
        after.is_sendable(),
        "a failed draft must be sendable again once it is fixed, or the only \
         way out is to retype it"
    );
    assert_eq!(
        after.to, draft.to,
        "the recipients did not survive the failure"
    );
    assert_eq!(
        after.body.text, draft.body.text,
        "the text did not survive the failure, which is the one outcome worse \
         than the send failing"
    );

    // ── FR-028: and it still knows which conversation it belongs to ──────
    //
    // A retry rebuilds the outgoing bytes from this row, so this field is
    // the whole of what makes the second attempt land where the first would
    // have. Losing it would show up only as a reply that started its own
    // thread, later, with nothing left to connect it back to the failure.
    assert_eq!(
        after.in_reply_to,
        Some(parent.id),
        "the draft forgot the message it answers, so a retry would start its \
         own conversation"
    );

    // ── And the reason is still somewhere ────────────────────────────────
    let row = queue
        .get(queued.id)
        .await
        .expect("get")
        .expect("a failed operation stays on the queue to be looked at");
    assert_eq!(
        row.last_error.as_deref(),
        Some("550 mailbox unavailable"),
        "the reason went with the attempt, so nothing can ever tell the \
         person why"
    );

    // ── And it is reachable from the draft, which is what a surface has ──
    //
    // #1487. A surface reopening a failed draft has a `DraftId` and nothing
    // else; the reason lives on a queue row keyed by target. Without this
    // there is no query from one to the other, which is why the reason was
    // durable and still unreachable.
    //
    // Read rather than copied onto the draft: one source of truth, and a
    // second copy is one that can disagree with the first about why a send
    // failed.
    assert_eq!(
        queue
            .last_failure_for(OperationTarget::Draft(draft.id))
            .await
            .expect("look for the failure")
            .as_deref(),
        Some("550 mailbox unavailable"),
        "the draft cannot reach the reason its own send failed"
    );

    // A draft whose send never failed has nothing to say, and must not
    // inherit somebody else's reason.
    let mut untroubled = a_draft(account.id);
    drafts.save(&mut untroubled).await.expect("save");
    assert_eq!(
        queue
            .last_failure_for(OperationTarget::Draft(untroubled.id))
            .await
            .expect("look for the failure"),
        None
    );
}

// ── The mirror row carries the send state (spec 003, T049) ──────────────────

/// What `messages.send_state` says for the row standing for `draft`.
async fn mirrored_state(connection: &Connection, draft: DraftId) -> Option<String> {
    postio_storage::sql::one(
        connection,
        "SELECT messages.send_state
               FROM messages
               JOIN drafts ON drafts.message_id = messages.id
              WHERE drafts.id = ?1",
        bind![draft.get()],
        |row| postio_storage::sql::RowExt::col::<Option<String>>(row, 0),
    )
    .await
    .expect("the draft has a mirror row")
}

#[tokio::test]
async fn the_mirror_row_carries_the_drafts_state_after_every_verb() {
    // The invariant the Outbox and Drafts predicates both rest on. They read
    // `messages.send_state`; the truth is `drafts.state`; and a denormalised
    // value that drifts from what it denormalises shows a message in the wrong
    // folder, or in neither.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    // `Mailbox::new` guesses the role from the path, so "Drafts" is one.
    test_support::mailbox(&connection, &account, "Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    assert_eq!(
        mirrored_state(&connection, draft.id).await.as_deref(),
        Some("editing"),
        "a draft being written is `editing` on both rows"
    );

    drafts.queue_send(&mut draft, at(0)).await.expect("send it");
    assert_eq!(
        mirrored_state(&connection, draft.id).await.as_deref(),
        Some("queued"),
        "pressing Send has to move the row the list draws, not only the draft"
    );

    for state in [
        DraftState::Sending,
        DraftState::Failed,
        DraftState::Unconfirmed,
    ] {
        drafts
            .set_state(draft.id, state)
            .await
            .expect("the drainer moves it");
        assert_eq!(
            mirrored_state(&connection, draft.id).await.as_deref(),
            Some(state.as_str()),
            "the drainer moved the draft to {state:?} and the mirror row did not follow"
        );
    }
}

#[tokio::test]
async fn an_ordinary_message_has_no_send_state_at_all() {
    // The column is NULL for everything that is not a draft, which is what
    // makes the partial index small and the Drafts exclusion cheap.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX").await;

    let mut message = Message::new(account.id, inbox.id, chrono::Utc::now());
    message.subject = Some("Ordinary mail".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("file it");

    let state: Option<String> = postio_storage::sql::one(
        &connection,
        "SELECT send_state FROM messages WHERE id = ?1",
        [message.id.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the row");
    assert_eq!(state, None, "mail that arrived is not a draft being sent");
}

// ── A draft is in exactly one of Drafts and the Outbox (FR-004) ─────────────

#[tokio::test]
async fn every_draft_state_puts_the_row_in_exactly_one_of_the_two_lists() {
    // The invariant, stated as a property over all five states rather than as
    // five separate tests: never both lists, never neither. "Neither" is the
    // failure #1491 reports -- a message you have sent that is nowhere -- and
    // "both" is the one a careless predicate produces.
    use postio_storage::repository::{ListQuery, ListScope};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let drafts_folder = test_support::mailbox(&connection, &account, "Drafts").await;
    let drafts = DraftRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    // The mirror row #166 wrote, found the way the composer finds it.
    let mirror: MessageId = postio_storage::sql::one(
        &connection,
        "SELECT message_id FROM drafts WHERE id = ?1",
        bind![draft.id.get()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
    )
    .await
    .map(MessageId::new)
    .expect("the draft has a mirror row");

    let listed = async |scope| {
        messages
            .page(&ListQuery {
                scope,
                limit: 50,
                after: None,
            })
            .await
            .expect("a page")
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>()
    };

    for state in [
        DraftState::Editing,
        DraftState::Queued,
        DraftState::Sending,
        DraftState::Failed,
        DraftState::Unconfirmed,
    ] {
        drafts.set_state(draft.id, state).await.expect("move it");

        let in_drafts = listed(ListScope::Mailbox(drafts_folder.id))
            .await
            .contains(&mirror);
        let in_outbox = listed(ListScope::Outbox(account.id))
            .await
            .contains(&mirror);

        assert!(
            in_drafts ^ in_outbox,
            "{state:?} is in {} lists; a draft belongs to exactly one",
            usize::from(in_drafts) + usize::from(in_outbox)
        );

        // And which one, because "exactly one" is satisfied by the wrong one.
        let expected_outbox = matches!(state, DraftState::Queued | DraftState::Sending);
        assert_eq!(
            in_outbox, expected_outbox,
            "{state:?} is in the wrong one of the two"
        );
    }
}

#[test]
fn the_outbox_has_a_constructor_like_every_other_scope() {
    // FR-016. The Outbox is reached the way every list is, and a caller that
    // has to spell out `limit` and `after` to name it is a caller reaching
    // past the type -- which is what the app suite was doing, and what a
    // second one would copy.
    use postio_storage::repository::{ListQuery, ListScope};

    let account = postio_model::AccountId::new(7);
    let query = ListQuery::outbox(account);

    assert_eq!(query.scope, ListScope::Outbox(account));
    assert_eq!(
        query.limit,
        ListQuery::flagged(account).limit,
        "a view's page is the same size as any other"
    );
    assert_eq!(query.after, None, "a fresh query starts at the newest");
}

#[tokio::test]
async fn a_scheduled_send_carries_its_due_time_and_an_immediate_one_does_not() {
    // FR-007. `Queued` covers two things a person means -- "as soon as you
    // can" and "on Thursday" -- and without the time they read identically.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    test_support::mailbox(&connection, &account, "Drafts").await;
    let drafts = DraftRepository::new(&connection);

    let due_at = async |draft: DraftId| -> Option<i64> {
        postio_storage::sql::one(
            &connection,
            "SELECT messages.send_at FROM messages
               JOIN drafts ON drafts.message_id = messages.id
              WHERE drafts.id = ?1",
            bind![draft.get()],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("the mirror row")
    };

    let mut now = a_draft(account.id);
    drafts.save(&mut now).await.expect("save");
    drafts.queue_send(&mut now, at(0)).await.expect("send now");
    assert_eq!(
        due_at(now.id).await,
        None,
        "an immediate send has no time anybody chose"
    );

    let mut later = a_draft(account.id);
    drafts.save(&mut later).await.expect("save");
    drafts
        .queue_send_at(&mut later, at(0), at(600))
        .await
        .expect("send later");
    assert!(
        due_at(later.id).await.is_some(),
        "a scheduled send has to carry the time, or the row cannot say it"
    );

    // And it stops being a plan the moment it is no longer merely waiting.
    drafts
        .set_state(later.id, DraftState::Sending)
        .await
        .expect("the drainer takes it");
    assert_eq!(
        due_at(later.id).await,
        None,
        "a send in flight has no future time to show"
    );
}

#[tokio::test]
async fn a_send_whose_operation_is_gone_stops_claiming_to_be_on_its_way() {
    // The state a real store was found in: `queued`, no operation, 25 hours
    // after the send. Nothing retries it, the Outbox lists it for ever, and
    // once `prune_settled` has removed the settled operation there is not
    // even a reason left to show.
    //
    // The cause is fixed where it happens -- a send that gives up now marks
    // its draft -- but that does nothing for the drafts already in this
    // state, and they cannot get out of it by themselves.
    use postio_storage::repository::OperationQueueRepository;

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    test_support::mailbox(&connection, &account, "Drafts").await;
    let drafts = DraftRepository::new(&connection);

    // One orphan, one healthy send, one draft still being written.
    let orphan = {
        let mut draft = a_draft(account.id);
        drafts.save(&mut draft).await.expect("save");
        drafts
            .queue_send(&mut draft, Utc::now())
            .await
            .expect("queue");
        // What the drainer's failure path used to leave behind, and what
        // `prune_settled` then finishes.
        let queued = OperationQueueRepository::new(&connection)
            .pending(account.id, Utc::now())
            .await
            .expect("the queue")
            .into_iter()
            .find(|op| op.target == postio_model::OperationTarget::Draft(draft.id))
            .expect("the send is queued");
        OperationQueueRepository::new(&connection)
            .delete(queued.id)
            .await
            .expect("delete the operation");
        draft.id
    };
    let healthy = {
        let mut draft = a_draft(account.id);
        drafts.save(&mut draft).await.expect("save");
        drafts
            .queue_send(&mut draft, Utc::now())
            .await
            .expect("queue");
        draft.id
    };
    let editing = {
        let mut draft = a_draft(account.id);
        drafts.save(&mut draft).await.expect("save")
    };

    let healed = drafts
        .fail_orphaned_sends(account.id)
        .await
        .expect("reconcile the orphans");
    assert_eq!(healed, 1, "exactly the one with nothing behind it");

    assert_eq!(
        drafts
            .get(orphan)
            .await
            .expect("get")
            .expect("still here")
            .state,
        DraftState::Failed,
        "the orphan still claims to be on its way"
    );
    assert_eq!(
        drafts
            .get(healthy)
            .await
            .expect("get")
            .expect("still here")
            .state,
        DraftState::Queued,
        "a send with an operation behind it was disturbed"
    );
    assert_eq!(
        drafts
            .get(editing)
            .await
            .expect("get")
            .expect("still here")
            .state,
        DraftState::Editing,
        "a draft being written is not a send at all"
    );

    // And it is idempotent, because it runs on every drain pass.
    assert_eq!(
        drafts.fail_orphaned_sends(account.id).await.expect("again"),
        0,
        "a second pass found something to do, so it would churn for ever"
    );
}

/// The rows a mailbox can never hold, counted so the sync can stop asking.
///
/// `upsert_batch` refuses the server's copy of a draft this client wrote, so
/// a draft that has been appended leaves the Drafts mailbox permanently one
/// row short of the server's `EXISTS`. `sync::resync` re-enumerates a mailbox
/// that is short, and without a count of what is deliberately absent it
/// re-enumerated Drafts on every pass for ever -- 27 held against 28
/// reported, the 28th being the draft, fetched and refused each time.
#[tokio::test]
async fn an_appended_drafts_server_copy_is_counted_as_refused() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, drafts_mailbox) = account_with_drafts(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
    let drafts = DraftRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    assert_eq!(
        messages
            .refused_rows_in(drafts_mailbox)
            .await
            .expect("count"),
        0,
        "a draft nobody has appended is held locally and refused nowhere"
    );

    let mut draft = a_draft(account.id);
    drafts.save(&mut draft).await.expect("save");
    drafts
        .set_server_copy(
            draft.id,
            Some(&postio_storage::repository::ServerCopyLocation {
                remote_id: postio_model::RemoteId::new("1707000000:42".to_owned()),
                uid: postio_model::Uid::new(42),
                uid_validity: postio_model::UidValidity::new(1_707_000_000),
            }),
        )
        .await
        .expect("record where the append landed");

    assert_eq!(
        messages
            .refused_rows_in(drafts_mailbox)
            .await
            .expect("count"),
        1,
        "once the server has a copy, that row is one the store will not hold"
    );
    assert_eq!(
        messages.refused_rows_in(inbox.id).await.expect("count"),
        0,
        "and it is the drafts mailbox that is short, not every mailbox"
    );
}
