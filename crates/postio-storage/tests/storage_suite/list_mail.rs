//! What a list has sent, read for "delete what the list sent" (#1931): one
//! account's messages from a `List-Id` or a sender, outside the folders that
//! are already the end of the road, and never a Flagged one.

use chrono::{TimeZone, Utc};
use postio_model::{EmailAddress, Message, MessageId, MailboxRole};
use postio_storage::Connection;
use postio_storage::repository::{ListMail, MailboxRepository, MessageRepository, SweepRepository};
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

async fn file(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    list: Option<&str>,
    from: &str,
    flagged: bool,
) -> MessageId {
    let mut message = Message::new(
        account.id,
        mailbox,
        Utc.with_ymd_and_hms(2026, 9, 26, 9, 0, 0).unwrap(),
    );
    message.list_id = list.map(str::to_owned);
    message.from = vec![EmailAddress::new(None::<String>, from)];
    if flagged { message.flags.insert(postio_model::Flag::Flagged); }
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("filed")
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
        .expect("role set");
    mailbox.id
}

#[tokio::test]
async fn a_list_id_names_its_messages_but_not_flagged_trashed_or_other_mail() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let trash = folder(&connection, &account, "Trash", MailboxRole::Trash).await;
    let list = Some("news.example.com");
    let one = file(&connection, &account, inbox, list, "a@news.example.com", false).await;
    let two = file(&connection, &account, inbox, list, "b@news.example.com", false).await;
    file(&connection, &account, inbox, list, "a@news.example.com", true).await;
    file(&connection, &account, trash, list, "a@news.example.com", false).await;
    file(&connection, &account, inbox, Some("other.example.com"), "a@x.example.com", false).await;
    file(&connection, &account, inbox, None, "friend@example.com", false).await;

    let found = SweepRepository::new(&connection)
        .list_mail(account.id, ListMail::ListId("news.example.com"), 100)
        .await
        .expect("read");
    let mut ids = found.ids.clone();
    ids.sort();
    assert_eq!(ids, vec![one, two]);
    assert!(!found.more);
}

#[tokio::test]
async fn without_a_list_id_the_exact_sender_names_it_not_their_domain() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let theirs = file(&connection, &account, inbox, None, "Shop@Example.com", false).await;
    file(&connection, &account, inbox, None, "friend@example.com", false).await;

    let found = SweepRepository::new(&connection)
        .list_mail(account.id, ListMail::Sender("shop@example.com"), 100)
        .await
        .expect("read");
    assert_eq!(found.ids, vec![theirs], "case does not matter, the domain does not widen it");
}

#[tokio::test]
async fn reading_a_lists_mail_is_a_bounded_number_of_statements_and_says_when_there_is_more() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    for _ in 0..7 {
        file(&connection, &account, inbox, Some("l.example.com"), "a@l.example.com", false).await;
    }
    let sweeps = SweepRepository::new(&connection);
    let counts = counted_async(|| async {
        let found = sweeps
            .list_mail(account.id, ListMail::ListId("l.example.com"), 5)
            .await
            .expect("read");
        assert_eq!(found.ids.len(), 5, "a page, not the mailbox");
        assert!(found.more);
    })
    .await;
    assert_eq!(counts.statements, 1, "one statement however long the list");
}
