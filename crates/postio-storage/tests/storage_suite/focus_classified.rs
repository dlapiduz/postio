//! What Focus has classified (spec 007, data-model.md "`focus_classified`"):
//! a record per message, stage and classifier version, and the catch-up's
//! read of what has none at the current version -- recent inbox mail whose
//! body is here, newest first (FR-141).

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::{BodyState, Message, MessageId};
use postio_storage::Connection;
use postio_storage::repository::{FocusClassifiedRepository, FocusStage, MessageRepository};
use postio_storage::test_support::{self, counting::counted_async};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap()
}

fn since() -> DateTime<Utc> {
    now() - TimeDelta::days(30)
}

/// A message in `mailbox`, received `ago` before now, with its body in
/// `state`.
async fn message(
    connection: &Connection,
    account: postio_model::AccountId,
    mailbox: postio_model::MailboxId,
    ago: TimeDelta,
    state: BodyState,
) -> MessageId {
    let messages = MessageRepository::new(connection);
    let mut message = Message::new(account, mailbox, now() - ago);
    let id = messages.create(&mut message).await.expect("a message");
    messages
        .set_body_state(id, state)
        .await
        .expect("its body state");
    id
}

/// The version `message` is recorded at for `stage`, if it is.
async fn recorded(connection: &Connection, message: MessageId, stage: &str) -> Option<i64> {
    postio_storage::sql::first(
        connection,
        "SELECT version FROM focus_classified WHERE message_id = ?1 AND stage = ?2",
        postio_storage::sql::bind![message.get(), stage],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("a read")
}

#[tokio::test]
async fn the_catch_up_reads_recent_inbox_mail_with_a_body_and_no_record_newest_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let hours = TimeDelta::hours;

    let newest = message(&connection, account.id, inbox, hours(1), BodyState::Full).await;
    let partial = message(&connection, account.id, inbox, hours(2), BodyState::Partial).await;
    let no_body = message(
        &connection,
        account.id,
        inbox,
        hours(3),
        BodyState::HeadersOnly,
    )
    .await;
    let _old = message(
        &connection,
        account.id,
        inbox,
        TimeDelta::days(40),
        BodyState::Full,
    )
    .await;
    let _archived = message(
        &connection,
        account.id,
        archive.id,
        hours(1),
        BodyState::Full,
    )
    .await;
    let _ = no_body;

    let focus = FocusClassifiedRepository::new(&connection);
    assert_eq!(
        focus
            .pending_bodies(inbox, since(), 1, 10)
            .await
            .expect("a read"),
        vec![newest, partial],
        "recent, in the inbox, with a body here, newest first"
    );
    assert_eq!(
        focus
            .pending_bodies(inbox, since(), 1, 1)
            .await
            .expect("a read"),
        vec![newest],
        "a batch at a time"
    );

    // Classified at this version: done. At another, or at another stage:
    // still to do.
    focus
        .record(&[newest], FocusStage::Body, 1)
        .await
        .expect("recorded");
    focus
        .record(&[partial], FocusStage::Body, 0)
        .await
        .expect("recorded");
    focus
        .record(&[partial], FocusStage::Filing, 1)
        .await
        .expect("recorded");
    assert_eq!(
        focus
            .pending_bodies(inbox, since(), 1, 10)
            .await
            .expect("a read"),
        vec![partial],
        "an older classifier's record, or the other stage's, is not this one's"
    );
}

#[tokio::test]
async fn a_record_is_one_per_message_and_stage_at_the_latest_version() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let id = message(
        &connection,
        account.id,
        inbox,
        TimeDelta::hours(1),
        BodyState::Full,
    )
    .await;
    let focus = FocusClassifiedRepository::new(&connection);

    focus
        .record(&[id], FocusStage::Body, 1)
        .await
        .expect("recorded");
    focus
        .record(&[id], FocusStage::Body, 2)
        .await
        .expect("recorded");
    focus
        .record(&[id], FocusStage::Filing, 1)
        .await
        .expect("recorded");

    assert_eq!(recorded(&connection, id, "body").await, Some(2));
    assert_eq!(recorded(&connection, id, "filing").await, Some(1));
}

#[tokio::test]
async fn the_catch_up_s_read_is_one_statement_that_seeks() {
    // FR-141: the catch-up runs in the background over the last 30 days of
    // an inbox, a batch at a time. Each batch is one statement, its rows are
    // the batch, and it walks the inbox's own index newest first.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    for hour in 1..=8 {
        message(
            &connection,
            account.id,
            inbox,
            TimeDelta::hours(hour),
            BodyState::Full,
        )
        .await;
    }
    let focus = FocusClassifiedRepository::new(&connection);

    let mut found = Vec::new();
    let counts = counted_async(|| async {
        found = focus
            .pending_bodies(inbox, since(), 1, 5)
            .await
            .expect("a read");
    })
    .await;
    assert_eq!(found.len(), 5);
    assert_eq!((counts.statements, counts.rows), (1, 5), "{counts:?}");

    let sql = FocusClassifiedRepository::explain_pending_bodies();
    let plan = test_support::plan(&connection, sql).await;
    assert!(
        !plan
            .lines()
            .any(|step| step.trim_start().starts_with("SCAN")),
        "{plan}"
    );
    assert!(
        !test_support::sorts(&plan),
        "newest first from the index:\n{plan}"
    );
}

#[tokio::test]
async fn of_the_bodies_that_landed_the_stage_takes_recent_inbox_mail_it_has_not_classified() {
    // The body stage hears of bodies as they land, in any folder, and takes
    // the same mail its catch-up would: in an inbox, recent, body here, and
    // not yet classified at this version. Newest first, in one statement.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let hours = TimeDelta::hours;
    let older = message(&connection, account.id, inbox, hours(3), BodyState::Full).await;
    let newer = message(&connection, account.id, inbox, hours(1), BodyState::Full).await;
    let done = message(&connection, account.id, inbox, hours(2), BodyState::Full).await;
    let archived = message(
        &connection,
        account.id,
        archive.id,
        hours(1),
        BodyState::Full,
    )
    .await;
    let old = message(
        &connection,
        account.id,
        inbox,
        TimeDelta::days(40),
        BodyState::Full,
    )
    .await;
    let focus = FocusClassifiedRepository::new(&connection);
    focus
        .record(&[done], FocusStage::Body, 1)
        .await
        .expect("recorded");

    let mut found = Vec::new();
    let counts = counted_async(|| async {
        found = focus
            .bodies_to_classify(&[older, newer, done, archived, old], since(), 1)
            .await
            .expect("a read");
    })
    .await;

    assert_eq!(found, vec![newer, older]);
    assert_eq!(counts.statements, 1, "{counts:?}");
}
