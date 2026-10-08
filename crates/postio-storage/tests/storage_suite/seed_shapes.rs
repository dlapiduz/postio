//! What the demo seeds promise their callers (`postio_storage::seed`): the
//! conversation lengths asked for come out threaded as asked, a second
//! account is a whole account of its own, and the helpers that dress a store
//! for a picture leave what a picture shows.

use chrono::{TimeZone, Utc};
use postio_model::{BodyState, MailboxRole};
use postio_storage::repository::{DraftRepository, MailboxRepository, MessageRepository};
use postio_storage::seed::{
    LONG_THREAD, leave_a_draft, queue_one_to_send, seed_conversations, seed_extra_account,
    seed_recent_with_bodies, seed_small, seed_small_downloaded, stamp_synced,
};
use postio_storage::test_support;

async fn count(connection: &postio_storage::Connection, sql: &str) -> i64 {
    postio_storage::sql::scalar(connection, sql, ())
        .await
        .expect("a count")
}

#[tokio::test]
async fn conversations_come_out_as_long_as_they_were_asked_to_be() {
    let database = test_support::memory().await;
    let report = seed_conversations(&database, 7, &[3, 1, 2]).await;
    assert_eq!(report.message_count, 6);
    let connection = database.connect().await.expect("checkout");
    assert_eq!(count(&connection, "SELECT count(*) FROM threads").await, 3);
    let mut lengths: Vec<i64> = postio_storage::sql::all(
        &connection,
        "SELECT message_count FROM threads ORDER BY message_count",
        (),
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("a read");
    lengths.sort();
    assert_eq!(lengths, vec![1, 2, 3]);
    assert_eq!(
        count(
            &connection,
            "SELECT count(*) FROM messages WHERE body_state = 'full'"
        )
        .await,
        6,
        "every seeded conversation message has its body stored"
    );

    let long = test_support::memory().await;
    let report = seed_conversations(&long, 7, LONG_THREAD).await;
    assert_eq!(report.message_count, 7);
    let connection = long.connect().await.expect("checkout");
    let unread = count(&connection, "SELECT unread_count FROM threads").await;
    assert_eq!(unread, 2, "the last two of a long thread are unread");
    assert!(report.mailbox(MailboxRole::Inbox).is_some());
}

#[tokio::test]
async fn a_second_account_is_a_whole_account_with_its_own_mail() {
    let database = test_support::memory().await;
    let first = seed_small(&database, 1).await;
    let second = seed_extra_account(&database, "Work", "work@example.org", 2).await;
    assert_ne!(first.account.id, second.account.id);
    assert_eq!(second.account.address.address, "work@example.org");
    assert!(second.message_count > 0);
    assert_eq!(second.mailboxes.len(), first.mailboxes.len());

    let connection = database.connect().await.expect("checkout");
    let mailboxes = MailboxRepository::new(&connection)
        .list_for_account(second.account.id)
        .await
        .expect("a read");
    assert_eq!(mailboxes.len(), second.mailboxes.len());
    let kept = count(
        &connection,
        &format!(
            "SELECT count(*) FROM messages WHERE account_id = {}",
            second.account.id.get()
        ),
    )
    .await;
    assert_eq!(kept as usize, second.message_count);
}

#[tokio::test]
async fn downloaded_seeds_have_bodies_and_the_rest_do_not() {
    let plain = test_support::memory().await;
    let report = seed_small(&plain, 3).await;
    let connection = plain.connect().await.expect("checkout");
    assert_eq!(
        count(
            &connection,
            "SELECT count(*) FROM messages WHERE body_state = 'full'"
        )
        .await,
        0,
        "{} messages, none downloaded",
        report.message_count
    );

    let downloaded = test_support::memory().await;
    let report = seed_small_downloaded(&downloaded, 3).await;
    let connection = downloaded.connect().await.expect("checkout");
    let full = count(
        &connection,
        "SELECT count(*) FROM messages WHERE body_state = 'full'",
    )
    .await;
    // A fixture with no text and no html has no body to store, and stays
    // unfetched; every other one is downloaded.
    assert!(
        full > 0 && (full as usize) <= report.message_count,
        "{full} of {} downloaded",
        report.message_count
    );
    assert!(
        full as usize + 5 >= report.message_count,
        "{full} of {}",
        report.message_count
    );
}

#[tokio::test]
async fn recent_mail_is_in_the_last_month_with_its_bodies() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 5).await;
    let now = Utc.with_ymd_and_hms(2026, 9, 29, 9, 0, 0).unwrap();
    let ids = seed_recent_with_bodies(&database, &report, 25, now, 11).await;
    assert_eq!(ids.len(), 25);

    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);
    for id in &ids {
        let message = messages.get(*id).await.expect("a read").expect("there");
        assert!(message.received_at <= now);
        assert!(now - message.received_at < chrono::TimeDelta::days(30));
        assert_eq!(message.sync.body_state, BodyState::Full);
        assert!(
            messages
                .body(*id)
                .await
                .expect("a read")
                .expect("a body")
                .text
                .is_some()
        );
    }
}

#[tokio::test]
async fn dressing_a_store_for_a_picture_leaves_what_a_picture_shows() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 9).await;
    let at = Utc.with_ymd_and_hms(2026, 9, 29, 8, 0, 0).unwrap();

    stamp_synced(&database, &report, at).await;
    queue_one_to_send(&database, report.account.id, at).await;
    leave_a_draft(&database, report.account.id).await;

    let connection = database.connect().await.expect("checkout");
    for mailbox in MailboxRepository::new(&connection)
        .list_for_account(report.account.id)
        .await
        .expect("a read")
    {
        assert_eq!(mailbox.last_synced_at, Some(at), "{}", mailbox.path);
    }
    let drafts = DraftRepository::new(&connection)
        .list_for_account(report.account.id)
        .await
        .expect("a read");
    assert_eq!(drafts.len(), 2);
    assert!(
        drafts
            .iter()
            .any(|draft| draft.subject == "Notes for Thursday")
    );
    assert!(
        drafts
            .iter()
            .any(|draft| draft.subject.starts_with("Re: maildir index"))
    );
}
