//! Drafts: CRUD and the autosave-friendly upsert.
//!
//! The bead's acceptance criterion is "draft upsert is idempotent under rapid
//! autosave".

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Attachment, Draft, DraftId, DraftKind, DraftState, EmailAddress, Message, MessageBody,
    MessageId, ThreadId,
};
use postio_storage::repository::{DraftRepository, MessageRepository};
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
