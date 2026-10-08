//! A woken snooze comes back at the top (spec 007, research R7, T093).
//!
//! The lists of a folder -- its conversations, its flat rows, and the
//! unified and Focus inboxes made of them -- are ordered by
//! `messages.sort_at`: `received_at` when a message is filed, and the wake
//! time when a snooze wakes. Search and the query views stay on
//! `received_at`, and so does a conversation's own chronology.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::{EmailAddress, Message, RfcMessageId};
use postio_storage::Connection;
use postio_storage::repository::{
    FocusListQuery, ListQuery, MessageRepository, ThreadListQuery, ThreadRepository,
    ThreadingRepository, UnifiedThreadListQuery,
};
use postio_storage::test_support;

fn at(hour: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap() + TimeDelta::hours(hour)
}

async fn file(
    connection: &Connection,
    (account, mailbox): (AccountId, MailboxId),
    hour: i64,
    rfc: &str,
) -> (MessageId, ThreadId) {
    let mut message = Message::new(account, mailbox, at(hour));
    message.rfc_message_id = Some(RfcMessageId::new(rfc));
    message.subject = Some(format!("About {rfc}"));
    message.from = vec![EmailAddress::new(None::<String>, "quinn@example.com")];
    let id = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    let threaded = ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("thread");
    (id, threaded.thread_id)
}

#[tokio::test]
async fn a_woken_snooze_lists_at_the_top_of_every_list_of_its_folder() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let here = (account.id, inbox);
    // The oldest of three, snoozed, and its time come and gone.
    let (snoozed, snoozed_thread) = file(&connection, here, 1, "<weir@example.com>").await;
    file(&connection, here, 2, "<sluice@example.com>").await;
    let (newest, _) = file(&connection, here, 3, "<lock@example.com>").await;
    let messages = MessageRepository::new(&connection);
    let now = Utc::now();
    messages
        .snooze(&[snoozed], now - TimeDelta::seconds(1))
        .await
        .expect("snoozed");
    messages
        .wake_due(account.id, now)
        .await
        .expect("the tick wakes it");

    let threads = ThreadRepository::new(&connection);
    let folder = threads
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("the folder's conversations");
    assert_eq!(
        folder
            .iter()
            .map(|row| row.latest.as_ref().expect("a row").id)
            .collect::<Vec<_>>()[..2],
        [snoozed, newest],
        "the woken conversation is the folder's top row, above mail that \
         arrived after it"
    );
    let flat = messages
        .page(&ListQuery::mailbox(inbox))
        .await
        .expect("the folder's messages");
    assert_eq!(flat[0].id, snoozed, "and the top of the flat list");
    let unified = threads
        .unified_page(&UnifiedThreadListQuery {
            limit: 10,
            after: None,
        })
        .await
        .expect("the unified inbox");
    assert_eq!(unified[0].row.id, Some(snoozed_thread), "and of Unified");
    let focus = threads
        .focus_page_at(
            &FocusListQuery {
                inboxes: vec![here],
                limit: 10,
                after: None,
            },
            0,
        )
        .await
        .expect("Focus's inbox");
    assert_eq!(
        focus[0].row.id,
        Some(snoozed_thread),
        "and of Focus's inbox"
    );
    let everything = threads
        .page(&ThreadListQuery::account(account.id))
        .await
        .expect("the account's conversations");
    assert_eq!(
        everything[0].id,
        Some(snoozed_thread),
        "and of the account's conversations"
    );

    // The message keeps its own time: the row says when it arrived, and the
    // conversation's chronology does not move.
    assert_eq!(flat[0].received_at, at(1));

    // A walk resumes by the new order: one row at a time, nothing repeats
    // and nothing is skipped.
    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let mut query = ThreadListQuery::in_mailbox(account.id, inbox).limit(1);
        query.after = after;
        let page = threads.page(&query).await.expect("a page");
        let Some(last) = page.last() else { break };
        after = Some(last.cursor());
        walked.extend(page.into_iter().map(|row| row.latest.expect("a row").id));
    }
    assert_eq!(
        walked,
        folder
            .iter()
            .map(|row| row.latest.as_ref().expect("a row").id)
            .collect::<Vec<_>>()
    );
}
