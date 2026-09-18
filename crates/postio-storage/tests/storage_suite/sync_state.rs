//! Per-mailbox sync state: round-trip, atomicity with the writes it describes,
//! and what a crash mid-sync leaves behind.

use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;
use postio_storage::sql::bind;

use postio_model::FullResyncReason;
use postio_model::{Generation, MailboxId, MailboxStatus, ModSeq, ResyncPlan, SyncState, Uid};
use postio_storage::repository::SyncStateRepository;
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, hour, 0, 0).unwrap()
}

/// Inserts a message straight into the table: this test is about the state
/// beside the messages, not about the message repository.
async fn insert_message(connection: &Connection, mailbox: MailboxId, uid: u32) {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, uid, received_at)
             SELECT account_id, id, ?2, 0 FROM mailboxes WHERE id = ?1",
            bind![mailbox.get(), i64::from(uid)],
        )
        .await
        .expect("insert a message");
}

async fn message_count(connection: &Connection, mailbox: MailboxId) -> i64 {
    postio_storage::sql::one(
        connection,
        "SELECT count(*) FROM messages WHERE mailbox_id = ?1",
        bind![mailbox.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("count messages")
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sync_state_round_trips_through_the_database() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let mut state = SyncState::never_synced(inbox, account.id);
    state.generation = Some(Generation::new(1_707_000_000));
    state.uid_next = Some(Uid::new(4_412));
    state.highest_mod_seq = Some(ModSeq::new(90_210));
    state.complete_full_sync(at(9));

    states.save(&state).await.expect("save");

    let stored = states.get(inbox).await.expect("get").expect("a row");
    assert_eq!(stored, state);
}

#[tokio::test]
async fn a_freshly_created_mailbox_has_never_synced() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let stored = states.get(inbox).await.expect("get").expect("a row");

    assert_eq!(stored, SyncState::never_synced(inbox, account.id));
    assert!(!stored.has_synced());
}

#[tokio::test]
async fn a_mailbox_that_is_not_there_has_no_state() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let states = SyncStateRepository::new(&connection);

    assert_eq!(states.get(MailboxId::new(404)).await.expect("get"), None);
}

#[tokio::test]
async fn saving_state_for_an_unpersisted_mailbox_is_refused() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let state = SyncState::never_synced(MailboxId::UNASSIGNED, account.id);

    assert!(matches!(
        states.save(&state).await,
        Err(postio_storage::Error::NotPersisted { entity: "mailbox" })
    ));
}

#[tokio::test]
async fn an_accounts_state_reads_in_one_query() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let states = SyncStateRepository::new(&connection);

    states
        .observe(inbox.id, &MailboxStatus::new(Generation::new(7)), at(9))
        .await
        .expect("observe");

    let all = states.list_for_account(account.id).await.expect("list");

    assert_eq!(all.len(), 2, "one row per mailbox, always");
    let inbox_state = all
        .iter()
        .find(|state| state.mailbox_id == inbox.id)
        .expect("the inbox");
    assert_eq!(inbox_state.generation, Some(Generation::new(7)));
    let archive_state = all
        .iter()
        .find(|state| state.mailbox_id == archive.id)
        .expect("the archive");
    assert!(!archive_state.has_synced());
}

// ---------------------------------------------------------------------------
// Observing and completing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn observing_persists_what_the_server_reported() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let status = MailboxStatus::new(Generation::new(1_707_000_000))
        .with_uid_next(Uid::new(4_412))
        .with_highest_mod_seq(ModSeq::new(90_210));
    let returned = states
        .observe(inbox, &status, at(9))
        .await
        .expect("observe");

    assert_eq!(returned.uid_next, Some(Uid::new(4_412)));
    assert_eq!(returned.last_seen_at, Some(at(9)));
    assert!(!returned.has_synced(), "selected is not synchronized");
    assert_eq!(states.get(inbox).await.expect("get"), Some(returned));
}

#[tokio::test]
async fn a_uid_validity_change_clears_the_stored_counters() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let first = MailboxStatus::new(Generation::new(1_707_000_000))
        .with_uid_next(Uid::new(4_412))
        .with_highest_mod_seq(ModSeq::new(90_210));
    states.observe(inbox, &first, at(9)).await.expect("observe");
    states
        .complete_full_sync(inbox, at(9))
        .await
        .expect("complete");

    let renumbered = states
        .observe(
            inbox,
            &MailboxStatus::new(Generation::new(1_800_000_000)),
            at(10),
        )
        .await
        .expect("observe");

    assert_eq!(renumbered.highest_mod_seq, None);
    assert_eq!(renumbered.uid_next, None);
    assert!(!renumbered.has_synced());
    assert_eq!(
        states.get(inbox).await.expect("get"),
        Some(renumbered),
        "and that is what is on disk, not just what was returned"
    );
}

#[tokio::test]
async fn the_plan_is_read_from_the_stored_state() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let status = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(100));

    assert_eq!(
        states.plan(inbox, &status).await.expect("plan"),
        ResyncPlan::Full(FullResyncReason::NeverSynced)
    );

    states
        .observe(inbox, &status, at(9))
        .await
        .expect("observe");
    states
        .complete_full_sync(inbox, at(9))
        .await
        .expect("complete");

    assert_eq!(
        states.plan(inbox, &status).await.expect("plan"),
        ResyncPlan::UpToDate
    );

    let moved_on = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(140));
    assert_eq!(
        states.plan(inbox, &moved_on).await.expect("plan"),
        ResyncPlan::Incremental {
            since: ModSeq::new(100)
        }
    );
}

#[tokio::test]
async fn planning_for_a_mailbox_that_is_gone_is_not_found() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let states = SyncStateRepository::new(&connection);

    assert!(matches!(
        states
            .plan(MailboxId::new(404), &MailboxStatus::new(Generation::new(7)))
            .await,
        Err(postio_storage::Error::NotFound {
            entity: "mailbox",
            id: 404
        })
    ));
}

#[tokio::test]
async fn resetting_takes_a_mailbox_back_to_never_synced() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);

    let status = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(100));
    states
        .observe(inbox, &status, at(9))
        .await
        .expect("observe");
    states
        .complete_full_sync(inbox, at(9))
        .await
        .expect("complete");

    let reset = states.reset(inbox).await.expect("reset");

    assert_eq!(reset, SyncState::never_synced(inbox, account.id));
    assert_eq!(states.get(inbox).await.expect("get"), Some(reset));
}

// ---------------------------------------------------------------------------
// Atomicity: the acceptance criteria of postio-gug
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_and_the_messages_it_describes_commit_together() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;

    let transaction = connection.transaction().await.expect("begin");
    insert_message(&transaction, inbox, 4_410).await;
    insert_message(&transaction, inbox, 4_411).await;
    let status = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(90_210));
    SyncStateRepository::new(&transaction)
        .observe(inbox, &status, at(9))
        .await
        .expect("observe");
    SyncStateRepository::new(&transaction)
        .complete_full_sync(inbox, at(9))
        .await
        .expect("complete");
    transaction.commit().await.expect("commit");

    assert_eq!(message_count(&connection, inbox).await, 2);
    let state = SyncStateRepository::new(&connection)
        .get(inbox)
        .await
        .expect("get")
        .expect("a row");
    assert!(state.has_synced());
    assert_eq!(state.highest_mod_seq, Some(ModSeq::new(90_210)));
}

#[tokio::test]
async fn a_crash_mid_sync_leaves_resumable_state_rather_than_a_lie() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    // A sync that got as far as writing some messages and advancing the
    // counters, then died before it could commit.
    let transaction = connection.transaction().await.expect("begin");
    insert_message(&transaction, inbox, 4_410).await;
    let status = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(90_210));
    SyncStateRepository::new(&transaction)
        .observe(inbox, &status, at(9))
        .await
        .expect("observe");
    SyncStateRepository::new(&transaction)
        .complete_full_sync(inbox, at(9))
        .await
        .expect("complete");
    drop(transaction); // the crash

    assert_eq!(message_count(&connection, inbox).await, 0);
    let states = SyncStateRepository::new(&connection);
    let state = states.get(inbox).await.expect("get").expect("a row");
    assert_eq!(
        state,
        SyncState::never_synced(inbox, account.id),
        "the state must never claim messages the database does not hold"
    );
    assert_eq!(
        states.plan(inbox, &status).await.expect("plan"),
        ResyncPlan::Full(FullResyncReason::NeverSynced),
        "so the next run picks the mailbox back up"
    );
}

#[tokio::test]
async fn state_never_advances_past_a_half_written_batch() {
    let database = test_support::memory().await;
    let mut connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;

    // First batch commits.
    let first = connection.transaction().await.expect("begin");
    insert_message(&first, inbox, 4_410).await;
    let status = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(100));
    SyncStateRepository::new(&first)
        .observe(inbox, &status, at(9))
        .await
        .expect("observe");
    SyncStateRepository::new(&first)
        .complete_full_sync(inbox, at(9))
        .await
        .expect("complete");
    first.commit().await.expect("commit");

    // Second batch dies partway.
    let second = connection.transaction().await.expect("begin");
    insert_message(&second, inbox, 4_411).await;
    let moved_on = MailboxStatus::new(Generation::new(7)).with_highest_mod_seq(ModSeq::new(140));
    SyncStateRepository::new(&second)
        .observe(inbox, &moved_on, at(10))
        .await
        .expect("observe");
    drop(second);

    let states = SyncStateRepository::new(&connection);
    let state = states.get(inbox).await.expect("get").expect("a row");
    assert_eq!(message_count(&connection, inbox).await, 1);
    assert_eq!(
        state.highest_mod_seq,
        Some(ModSeq::new(100)),
        "the rolled-back MODSEQ would have skipped UID 4411 forever"
    );
    assert_eq!(
        states.plan(inbox, &moved_on).await.expect("plan"),
        ResyncPlan::Incremental {
            since: ModSeq::new(100)
        }
    );
}

#[tokio::test]
async fn deleting_a_mailbox_takes_its_state_with_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let states = SyncStateRepository::new(&connection);
    states
        .observe(inbox, &MailboxStatus::new(Generation::new(7)), at(9))
        .await
        .expect("observe");

    connection
        .execute("DELETE FROM mailboxes WHERE id = ?1", [inbox.get()])
        .await
        .expect("delete the mailbox");

    assert_eq!(states.get(inbox).await.expect("get"), None);
    assert!(
        states
            .list_for_account(account.id)
            .await
            .expect("list")
            .is_empty()
    );
}
