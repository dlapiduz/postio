//! Auto-emptying Junk and Trash (#1932): which mail is old enough, found a
//! page at a time, and what purging it leaves in the queue.

use chrono::{Duration, TimeZone, Utc};
use postio_model::{Flag, MailboxRole, Message, MessageId, Operation};
use postio_storage::Connection;
use postio_storage::repository::{
    ExpiryRepository, MailboxRepository, MessageRepository, OperationQueueRepository,
};
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap()
}

async fn folder(
    connection: &Connection,
    account: &postio_model::Account,
    path: &str,
    role: MailboxRole,
) -> postio_model::MailboxId {
    let mut mailbox = test_support::mailbox(connection, account, path).await;
    mailbox.role = role;
    MailboxRepository::new(connection)
        .update(&mailbox)
        .await
        .expect("role");
    mailbox.id
}

async fn file(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    days_old: i64,
    flagged: bool,
) -> MessageId {
    let mut message = Message::new(account.id, mailbox, now() - Duration::days(days_old));
    if flagged {
        message.flags.insert(Flag::Flagged);
    }
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("filed")
}

#[tokio::test]
async fn only_old_unflagged_mail_in_the_asked_folder_kind_is_found() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let junk = folder(&connection, &account, "Junk", MailboxRole::Junk).await;
    let trash = folder(&connection, &account, "Trash", MailboxRole::Trash).await;
    let old_junk = file(&connection, &account, junk, 40, false).await;
    file(&connection, &account, junk, 5, false).await;
    file(&connection, &account, junk, 400, true).await;
    file(&connection, &account, trash, 40, false).await;
    file(&connection, &account, inbox, 400, false).await;

    let found = ExpiryRepository::new(&connection)
        .expired_in(
            account.id,
            MailboxRole::Junk,
            now() - Duration::days(30),
            100,
        )
        .await
        .expect("read");
    assert_eq!(found.messages, vec![(junk, old_junk)]);
    assert!(!found.more);
}

#[tokio::test]
async fn a_pass_reads_a_page_in_one_statement_oldest_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let trash = folder(&connection, &account, "Trash", MailboxRole::Trash).await;
    let oldest = file(&connection, &account, trash, 90, false).await;
    for days in 40..46 {
        file(&connection, &account, trash, days, false).await;
    }
    let expiry = ExpiryRepository::new(&connection);
    let counts = counted_async(|| async {
        let found = expiry
            .expired_in(
                account.id,
                MailboxRole::Trash,
                now() - Duration::days(30),
                3,
            )
            .await
            .expect("read");
        assert_eq!(found.messages.len(), 3, "a page, not the folder");
        assert!(found.more);
        assert_eq!(found.messages[0].1, oldest, "oldest first");
    })
    .await;
    assert_eq!(counts.statements, 1);
    assert_eq!(counts.rows, 4, "a page and one to know there is more");
}

#[tokio::test]
async fn purging_hides_the_mail_and_queues_one_purge_each() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let trash = folder(&connection, &account, "Trash", MailboxRole::Trash).await;
    let one = file(&connection, &account, trash, 40, false).await;
    let two = file(&connection, &account, trash, 41, false).await;

    postio_storage::actions::purge(&connection, account.id, trash, &[one, two], now())
        .await
        .expect("purged");

    let found = ExpiryRepository::new(&connection)
        .expired_in(account.id, MailboxRole::Trash, now(), 100)
        .await
        .expect("read");
    assert!(found.messages.is_empty(), "hidden, so not found again");
    let pending = OperationQueueRepository::new(&connection)
        .pending(account.id, now())
        .await
        .expect("queue");
    let purges = pending
        .iter()
        .filter(|row| row.operation == Operation::Purge { mailbox: trash })
        .count();
    assert_eq!(purges, 2);
}
