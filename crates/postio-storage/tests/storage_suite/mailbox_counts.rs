//! The cached mailbox counts, and the invariant that they have a writer.
//!
//! `mailboxes.total_count` is not decoration. `postio-runtime`'s store answers
//! the message list's "how many rows are there" from that column rather than
//! by counting, because the list asks with every page and `count(*)` over a
//! folder is linear in its size. The list is a `GListModel`: a total of zero
//! means `GtkListView` asks for no pages at all, so a wrong count here is not
//! a wrong number on screen, it is an empty mailbox.
//!
//! That is what `postio-qhz.7` turned out to be. On a live account with 81,716
//! messages in the store, every `total_count` was 0 and the list drew nothing
//! in every folder — while `select count(*) from messages` returned the real
//! number. `MailboxRepository::recount` existed, was tested, and had one
//! production caller in the whole workspace (the Sent box after a send). The
//! column was derived data with no owner.
//!
//! It has one now: triggers on `messages` maintain it, so every writer keeps
//! it true without knowing it exists. These tests are about that invariant
//! rather than about any one repository method — each goes through a *caller*
//! and reads the cached row back without recounting, because a test that
//! recounted first would pass against the bug.

use chrono::{TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    Account, Flag, FlagSet, MailboxId, MailboxRole, Message, MessageId, Uid, UidValidity,
};
use postio_storage::repository::{FlagSource, MailboxRepository, MessageRepository};
use postio_storage::test_support;

/// The cached counts as the sidebar reads them: off the mailbox row, with
/// nothing recounted on the way.
async fn cached(connection: &Connection, mailbox: MailboxId) -> (u32, u32, u32) {
    let counts = MailboxRepository::new(connection)
        .counts(mailbox)
        .await
        .expect("read the cached counts")
        .expect("the mailbox exists");
    (counts.total, counts.unread, counts.flagged)
}

/// A message with only what the counts care about.
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

/// Writes `count` messages and hands back their ids.
async fn write(
    connection: &Connection,
    account: &Account,
    mailbox: MailboxId,
    flags: &[&[Flag]],
) -> Vec<MessageId> {
    let messages = MessageRepository::new(connection);
    // A loop rather than `map().collect()`: the writes await, and a closure
    // cannot.
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

// ---------------------------------------------------------------------------
// The counts follow the messages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn writing_messages_moves_the_counts_without_anyone_recounting() {
    // The failure this is about: a sync writes tens of thousands of rows and
    // the list still believes the folder is empty.
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
    // The sync path. `upsert_batch` inserts what is new and updates what is
    // already there, in one transaction, and a second pass over the same UIDs
    // must not double the folder.
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

    // The same UIDs again — an interrupted pass resuming, which is ordinary.
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
    // `deleted_locally` is what makes delete feel instant: the row stays and
    // the list stops showing it. The counts have to agree with the list, or
    // the sidebar promises rows the folder will not produce — and undo has to
    // put the number back.
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
    // The counts are maintained by arithmetic, so a write that sets a column
    // to what it already held is the case that would drift.
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

// ---------------------------------------------------------------------------
// Repairing a store written before the column had a writer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn counts_that_have_drifted_to_zero_are_repairable_without_a_sync() {
    // `postio-qhz.7`: a store of 81,716 messages whose every `total_count`
    // was 0, so the list drew nothing in every folder while
    // `select count(*) from messages` returned the real number.
    //
    // The triggers are what stop that arising now, and the rest of this file
    // is about them. This is the other half: if a count ever does drift — a
    // bulk write that went round the triggers, a database edited by hand —
    // `recount` puts it right from the rows themselves, offline. A populated
    // local store is populated whether or not a server can be reached, so the
    // repair must never need one.
    //
    // This used to freeze an old migration prefix and prove that migrating to
    // head repaired the counts. There are no old stores and no migration
    // history any more (ADR 0020), and a fresh schema has the triggers from
    // its first statement, so the drift is induced directly instead.
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

    // Drift, spelled out: the rows are all there and the column is a lie.
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

    // And the triggers keep them from there on, without a second recount.
    let mut sixth = a_message(&account, inbox, 6, &[]).await;
    MessageRepository::new(&connection)
        .create(&mut sixth)
        .await
        .expect("one more");
    assert_eq!(cached(&connection, inbox).await, (6, 4, 1));
}

#[tokio::test]
async fn a_seeded_store_still_agrees_with_a_recount() {
    // The counts now have two writers — the triggers, and `recount` as the
    // repair path. They must not disagree, or which one ran last decides what
    // the sidebar says.
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

// ── What the sidebar draws beside Drafts and the Outbox (spec 003, T066) ────

/// Seeds one draft per state and returns the account.
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
    // Three numbers from one read. The Outbox has no mailbox row to hold a
    // cached count -- it is not a mailbox -- and the Drafts badge can no
    // longer be `total_count`, which still counts every message row filed
    // there including the ones in flight.
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
    // The ordinary state, and the one that keeps the Outbox row hidden.
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
    // SC-008, and Principle V's "counts, not timings". The sidebar refreshes
    // on every arrival, so a read that grew with the mailbox would be paid
    // for on the surface redrawn most often. `idx_messages_send_state` is
    // partial on exactly this predicate, so the account's mail is not touched.
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

    // A mailbox's worth of ordinary mail, none of it a draft.
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
    // FR-024. The whole point of counting attention separately: the number
    // goes down when you deal with one. A retry that left it at 2 would make
    // the badge a thing to ignore.
    use postio_model::{Draft, DraftState};
    use postio_storage::repository::DraftRepository;

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = an_account_mid_send(&connection).await;
    let mailboxes = MailboxRepository::new(&connection);

    let before = mailboxes.draft_counts(account).await.expect("counts");
    assert_eq!((before.outbox, before.attention), (2, 2));

    // The gesture: open the failed one and send it again. `queue_send` is
    // what the composer calls, so this is that path and not a shortcut.
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
    // FR-025. #1487 computed the reason, wrote it to the queue row and
    // carried it up the engine's report, where nobody read it; `compose.rs`
    // reads it now and says "Not sent — {reason}". This is the half that can
    // be asserted without a display: the reason survives on the row for the
    // composer to find when the person comes back to it.
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

    // Read back the way `compose.rs` reads it: by the draft it is about,
    // which is all the composer has when somebody reopens the row.
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

/// How many of `mailbox`'s visible messages are still owed a body, as the
/// cached column says.
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
    // #1612: whether a search's corpus is complete was a walk of every
    // message in the account once backfill had finished -- the steady state
    // under ADR 0016 -- because `body_state` is in no index. A count the
    // triggers keep, like the four beside it, answers it by reading one row
    // per folder. This proves the count follows every write that moves it.
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
