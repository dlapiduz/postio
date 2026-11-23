//! `mailboxes.total_count` feeds the message list's row total, and zero means the list asks for no
//! pages. Triggers on `messages` maintain it, so each test goes through a caller and reads the
//! cached row without recounting (`postio-qhz.7`).

use chrono::{TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    Account, Flag, FlagSet, MailboxId, MailboxRole, Message, MessageId, Uid, UidValidity,
};
use postio_storage::repository::{FlagSource, MailboxRepository, MessageRepository};
use postio_storage::test_support;

async fn cached(connection: &Connection, mailbox: MailboxId) -> (u32, u32, u32) {
    let counts = MailboxRepository::new(connection)
        .counts(mailbox)
        .await
        .expect("read the cached counts")
        .expect("the mailbox exists");
    (counts.total, counts.unread, counts.flagged)
}

async fn a_message(account: &Account, mailbox: MailboxId, uid: u32, flags: &[Flag]) -> Message {
    let at = Utc
        .timestamp_opt(1_770_000_000 + i64::from(uid), 0)
        .unwrap();
    let mut message = Message::new(account.id, mailbox, at);
    message.subject = Some(format!("Message {uid}"));
    message.flags = flags.iter().cloned().collect::<FlagSet>();
    message.server.uid = Some(Uid::new(uid));
    message.server.uid_validity = Some(UidValidity::new(7));
    message
}

async fn write(
    connection: &Connection,
    account: &Account,
    mailbox: MailboxId,
    flags: &[&[Flag]],
) -> Vec<MessageId> {
    let messages = MessageRepository::new(connection);
    let mut written = Vec::new();
    for (index, flags) in flags.iter().enumerate() {
        let mut message = a_message(account, mailbox, index as u32 + 1, flags).await;
        written.push(
            messages
                .create(&mut message)
                .await
                .expect("write a message"),
        );
    }
    written
}

#[tokio::test]
async fn writing_messages_moves_the_counts_without_anyone_recounting() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    assert_eq!(cached(&connection, inbox).await, (0, 0, 0), "a new folder");

    write(
        &connection,
        &account,
        inbox,
        &[&[], &[Flag::Seen], &[Flag::Seen, Flag::Flagged]],
    )
    .await;

    assert_eq!(
        cached(&connection, inbox).await,
        (3, 1, 1),
        "three messages, one unread, one flagged — and nothing called recount"
    );
}

#[tokio::test]
async fn a_batch_upsert_counts_each_row_once() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut batch: Vec<Message> = Vec::new();
    for uid in 1..=4 {
        batch.push(a_message(&account, inbox, uid, &[]).await);
    }
    messages.upsert_batch(&mut batch).await.expect("first pass");
    assert_eq!(cached(&connection, inbox).await, (4, 4, 0));

    let mut again: Vec<Message> = Vec::new();
    for uid in 1..=4 {
        again.push(a_message(&account, inbox, uid, &[Flag::Seen]).await);
    }
    messages
        .upsert_batch(&mut again)
        .await
        .expect("second pass");
    assert_eq!(
        cached(&connection, inbox).await,
        (4, 0, 0),
        "the same four messages, now read — not eight messages"
    );
}

#[tokio::test]
async fn reading_a_message_moves_the_unread_count() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let ids = write(&connection, &account, inbox, &[&[], &[]]).await;
    assert_eq!(cached(&connection, inbox).await, (2, 2, 0));

    MessageRepository::new(&connection)
        .set_flags(
            ids[0],
            &[Flag::Seen].into_iter().collect(),
            FlagSource::Local,
        )
        .await
        .expect("mark it read");

    assert_eq!(
        cached(&connection, inbox).await,
        (2, 1, 0),
        "reading a message does not remove it from the folder"
    );
}

#[tokio::test]
async fn moving_a_message_moves_its_count_with_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

    let ids = write(&connection, &account, inbox, &[&[], &[Flag::Flagged]]).await;
    assert_eq!(cached(&connection, inbox).await, (2, 2, 1));

    MessageRepository::new(&connection)
        .move_to(&ids[1..], archive)
        .await
        .expect("archive it");

    assert_eq!(
        cached(&connection, inbox).await,
        (1, 1, 0),
        "the folder it left"
    );
    assert_eq!(
        cached(&connection, archive).await,
        (1, 1, 1),
        "and the one it joined"
    );
}

#[tokio::test]
async fn a_message_hidden_locally_leaves_the_counts_and_comes_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let ids = write(&connection, &account, inbox, &[&[], &[Flag::Flagged]]).await;
    let messages = MessageRepository::new(&connection);

    messages
        .set_deleted_locally(&ids[1..], true)
        .await
        .expect("hide");
    assert_eq!(cached(&connection, inbox).await, (1, 1, 0));

    messages
        .set_deleted_locally(&ids[1..], false)
        .await
        .expect("undo");
    assert_eq!(
        cached(&connection, inbox).await,
        (2, 2, 1),
        "undo restores it"
    );
}

#[tokio::test]
async fn deleting_a_row_takes_it_out_of_the_counts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let ids = write(&connection, &account, inbox, &[&[], &[]]).await;

    MessageRepository::new(&connection)
        .delete(&ids[..1])
        .await
        .expect("expunge it");

    assert_eq!(cached(&connection, inbox).await, (1, 1, 0));
}

#[tokio::test]
async fn hiding_a_message_twice_does_not_take_it_out_twice() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let ids = write(&connection, &account, inbox, &[&[], &[]]).await;
    let messages = MessageRepository::new(&connection);

    messages
        .set_deleted_locally(&ids[..1], true)
        .await
        .expect("hide");
    messages
        .set_deleted_locally(&ids[..1], true)
        .await
        .expect("hide it again");

    assert_eq!(cached(&connection, inbox).await, (1, 1, 0));
}

#[tokio::test]
async fn counts_that_have_drifted_to_zero_are_repairable_without_a_sync() {
    // If a count ever drifts (a bulk write round the triggers, a hand-edited database), `recount`
    // repairs it from the rows, offline.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    write(
        &connection,
        &account,
        inbox,
        &[&[], &[], &[], &[Flag::Seen], &[Flag::Seen, Flag::Flagged]],
    )
    .await;
    assert_eq!(cached(&connection, inbox).await, (5, 3, 1));

    connection
        .execute(
            "UPDATE mailboxes SET total_count = 0, unread_count = 0, flagged_count = 0",
            (),
        )
        .await
        .expect("zero the counts");
    assert_eq!(
        cached(&connection, inbox).await,
        (0, 0, 0),
        "the state the bug leaves behind"
    );

    MailboxRepository::new(&connection)
        .recount(inbox)
        .await
        .expect("recount");
    assert_eq!(
        cached(&connection, inbox).await,
        (5, 3, 1),
        "the store repairs its counts from its own rows"
    );

    let mut sixth = a_message(&account, inbox, 6, &[]).await;
    MessageRepository::new(&connection)
        .create(&mut sixth)
        .await
        .expect("one more");
    assert_eq!(cached(&connection, inbox).await, (6, 4, 1));
}

#[tokio::test]
async fn a_seeded_store_still_agrees_with_a_recount() {
    // The counts have two writers, the triggers and `recount`; they must not disagree.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let report = postio_storage::seed::seed_small(&database, 12).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");

    let before = cached(&connection, inbox.id).await;
    let recounted = MailboxRepository::new(&connection)
        .recount(inbox.id)
        .await
        .expect("recount");

    assert_eq!(
        before,
        (recounted.total, recounted.unread, recounted.flagged),
        "the triggers and the recount have to mean the same thing"
    );
}

async fn an_account_mid_send(connection: &Connection) -> postio_model::AccountId {
    use postio_model::{Draft, DraftState};
    use postio_storage::repository::DraftRepository;

    let account = test_support::account(connection).await;
    test_support::mailbox(connection, &account, "Drafts").await;
    let drafts = DraftRepository::new(connection);

    for state in [
        DraftState::Editing,
        DraftState::Editing,
        DraftState::Queued,
        DraftState::Sending,
        DraftState::Failed,
        DraftState::Unconfirmed,
    ] {
        let mut draft = Draft::new(account.id);
        draft.subject = format!("{state:?}");
        drafts.save(&mut draft).await.expect("save");
        drafts.set_state(draft.id, state).await.expect("move it");
    }
    account.id
}

#[tokio::test]
async fn the_sidebar_counts_what_is_on_its_way_and_what_needs_a_person() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account_mid_send(&connection).await;

    let counts = MailboxRepository::new(&connection)
        .draft_counts(account)
        .await
        .expect("counts");

    assert_eq!(counts.outbox, 2, "queued and sending are on their way");
    assert_eq!(
        counts.attention, 2,
        "failed and unconfirmed have stopped and need a person (FR-022)"
    );
    assert_eq!(
        counts.drafts, 4,
        "what Drafts shows: everything that is not in flight -- two being \
         written, and the two that need attention"
    );
}

#[tokio::test]
async fn an_account_sending_nothing_counts_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;

    let counts = MailboxRepository::new(&connection)
        .draft_counts(account.id)
        .await
        .expect("counts");
    assert_eq!((counts.outbox, counts.attention, counts.drafts), (0, 0, 0));
}

#[tokio::test]
async fn the_draft_counts_cost_the_same_however_much_mail_the_account_has() {
    // Counts, not timings. A statement and row count cannot see a walk of the account's mail, so
    // `draft_counts_read_only_the_account_s_drafts_from_an_index` asks the planner (#1614).
    use postio_storage::test_support::counting::{counted_async, install};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account_mid_send(&connection).await;
    let inbox = test_support::mailbox(
        &connection,
        &test_support::account(&connection).await,
        "INBOX",
    )
    .await;
    install(&connection);

    let mailboxes = MailboxRepository::new(&connection);
    let _ = mailboxes.draft_counts(account).await.expect("warm");
    let small = counted_async(|| async {
        mailboxes.draft_counts(account).await.expect("counts");
    })
    .await;

    for _ in 0..400 {
        let mut message = postio_model::Message::new(account, inbox.id, chrono::Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("file it");
    }

    let large = counted_async(|| async {
        mailboxes.draft_counts(account).await.expect("counts");
    })
    .await;

    assert_eq!(
        small.statements, large.statements,
        "the count query changed shape with the mailbox"
    );
    assert_eq!(
        small.rows, large.rows,
        "the count read more rows once the account had mail: {small:?} then {large:?}"
    );
}

#[tokio::test]
async fn retrying_a_failed_draft_moves_it_to_the_outbox_and_lowers_what_needs_you() {
    use postio_model::{Draft, DraftState};
    use postio_storage::repository::DraftRepository;

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account_mid_send(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let before = mailboxes.draft_counts(account).await.expect("counts");
    assert_eq!((before.outbox, before.attention), (2, 2));

    let drafts = DraftRepository::new(&connection);
    let failed = drafts
        .by_state(DraftState::Failed)
        .await
        .expect("by_state")
        .into_iter()
        .next()
        .expect("one failed draft");
    let mut failed = drafts
        .get(failed.id)
        .await
        .expect("get")
        .expect("the draft");
    drafts
        .queue_send(&mut failed, chrono::Utc::now())
        .await
        .expect("send it again");

    let after = mailboxes.draft_counts(account).await.expect("counts");
    assert_eq!(
        after.attention, 1,
        "retrying one of two should leave one needing a person"
    );
    assert_eq!(after.outbox, 3, "and it is on its way now");
    assert_eq!(
        after.drafts,
        before.drafts - 1,
        "Drafts holds one fewer, because the row moved rather than copied"
    );
    let _ = Draft::new(account);
}

#[tokio::test]
async fn a_failed_send_keeps_the_reason_the_composer_shows() {
    use postio_model::{Draft, DraftState, Operation, OperationTarget};
    use postio_storage::repository::{DraftRepository, OperationQueueRepository};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    test_support::mailbox(&connection, &account, "Drafts").await;

    let drafts = DraftRepository::new(&connection);
    let mut draft = Draft::new(account.id);
    draft.subject = "Re: the contract".to_owned();
    drafts.save(&mut draft).await.expect("save");
    let queued = drafts
        .queue_send(&mut draft, chrono::Utc::now())
        .await
        .expect("send");

    let queue = OperationQueueRepository::new(&connection);
    queue
        .mark_failed(queued.id, chrono::Utc::now(), "550 mailbox unavailable")
        .await
        .expect("the server refuses it");
    drafts
        .set_state(draft.id, DraftState::Failed)
        .await
        .expect("the drainer gives up");

    let reason = queue
        .last_failure_for(OperationTarget::Draft(draft.id))
        .await
        .expect("last_failure_for");
    assert!(
        reason
            .as_deref()
            .is_some_and(|reason| reason.contains("550")),
        "the server's own words have to survive for the composer to show \
         them -- \"something went wrong\" is what FR-025 exists to prevent: \
         {reason:?}"
    );
    let _ = Operation::Send { draft: draft.id };
}

async fn owed(connection: &Connection, mailbox: MailboxId) -> i64 {
    postio_storage::sql::one(
        connection,
        "SELECT bodies_owed FROM mailboxes WHERE id = ?1",
        [mailbox.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the cached column")
}

#[tokio::test]
async fn a_folder_knows_how_many_of_its_messages_are_owed_a_body() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let ids = write(&connection, &account, inbox, &[&[], &[], &[]]).await;
    assert_eq!(
        owed(&connection, inbox).await,
        3,
        "a new message has no body yet"
    );

    connection
        .execute(
            "UPDATE messages SET body_state = 'full' WHERE id = ?1",
            [ids[0].get()],
        )
        .await
        .expect("a body arrives");
    assert_eq!(
        owed(&connection, inbox).await,
        2,
        "a body that arrived is not owed"
    );

    connection
        .execute(
            "UPDATE messages SET body_state = 'headers_only' WHERE id = ?1",
            [ids[1].get()],
        )
        .await
        .expect("headers only");
    assert_eq!(
        owed(&connection, inbox).await,
        2,
        "headers only is still owed"
    );

    MessageRepository::new(&connection)
        .move_to(&[ids[1]], archive)
        .await
        .expect("moved");
    assert_eq!(
        owed(&connection, inbox).await,
        1,
        "a message that left is not owed here"
    );
    assert_eq!(
        owed(&connection, archive).await,
        1,
        "and is owed where it went"
    );

    connection
        .execute(
            "UPDATE messages SET deleted_locally = 1 WHERE id = ?1",
            [ids[2].get()],
        )
        .await
        .expect("hidden pending a remote delete");
    assert_eq!(
        owed(&connection, inbox).await,
        0,
        "a hidden message is not searched"
    );

    connection
        .execute("DELETE FROM messages WHERE id = ?1", [ids[1].get()])
        .await
        .expect("deleted");
    assert_eq!(
        owed(&connection, archive).await,
        0,
        "a deleted message is owed nothing"
    );
}

#[tokio::test]
async fn draft_counts_read_only_the_account_s_drafts_from_an_index() {
    // #1614: the planner seeked the account and read every message row for `deleted_locally`; the
    // gate is a range on `send_state` from an index that covers the rest.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let sql =
        postio_storage::repository::MailboxRepository::new(&connection).explain_draft_counts();
    let plan: Vec<String> = postio_storage::sql::all(
        &connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        [account.id.get()],
        |row| postio_storage::sql::RowExt::col(row, 3),
    )
    .await
    .expect("a plan");
    let reads_rows: Vec<&String> = plan
        .iter()
        .filter(|step| !(step.contains("COVERING INDEX") || step.starts_with("USE ")))
        .collect();
    assert!(
        reads_rows.is_empty(),
        "draft counts read message rows: {reads_rows:?}\n{plan:#?}"
    );
    assert!(
        plan.iter().any(|step| step.contains("send_state")),
        "draft counts walk the whole account instead of its drafts:\n{plan:#?}"
    );
}
