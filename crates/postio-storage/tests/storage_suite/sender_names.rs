//! The name the user gave a person is the name their mail carries
//! (specs/005-contacts FR-032, research R7).
//!
//! Keyed by address, never by the header's words: mail from any of a named
//! person's addresses shows that name in the list and in a conversation's
//! participants, while mail from an address nobody owns keeps whatever its
//! header said -- which is what stops a forged display name borrowing a
//! trusted one. A person the user deleted lends their name to nothing.

use chrono::{TimeZone, Utc};

use postio_model::{Account, ContactId, EmailAddress, MailboxId, Message, MessageId, Thread};
use postio_storage::Connection;
use postio_storage::repository::{
    ContactRepository, ListQuery, ListScope, MessageRepository, ThreadListQuery,
    ThreadRepository,
};
use postio_storage::test_support;

async fn from(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    thread: postio_model::ids::ThreadId,
    subject: &str,
    name: &str,
    email: &str,
    day: u32,
) -> MessageId {
    let at = Utc.with_ymd_and_hms(2026, 3, day, 12, 0, 0).unwrap();
    let mut message = Message::new(account.id, inbox, at);
    message.from = vec![EmailAddress::new(Some(name), email)];
    message.subject = Some(subject.into());
    let id = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("a message");
    ThreadRepository::new(connection)
        .add_message(thread, id)
        .await
        .expect("threaded");
    id
}

fn titled(account: &Account, subject: &str) -> Thread {
    let mut thread = Thread::new(account.id);
    thread.subject = Some(subject.into());
    thread
}

struct Mail {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    inbox: MailboxId,
    account: Account,
    ada: ContactId,
}

/// Ada writes from two addresses under two header names, and a stranger
/// writes claiming to be her; the user names Ada.
async fn mail() -> Mail {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let thread = ThreadRepository::new(&connection)
        .create(&mut titled(&account, "Engines"))
        .await
        .expect("a thread");
    from(&connection, &account, inbox, thread, "Engines", "A. L.", "ada@work.example", 1).await;
    from(&connection, &account, inbox, thread, "Engines", "ada", "ada@home.example", 2).await;
    let stranger = ThreadRepository::new(&connection)
        .create(&mut titled(&account, "Urgent"))
        .await
        .expect("another thread");
    from(
        &connection,
        &account,
        inbox,
        stranger,
        "Urgent",
        "Ada Lovelace",
        "not-ada@example.net",
        3,
    )
    .await;
    let ada = ContactRepository::new(&connection)
        .create(
            Some("Countess of Lovelace"),
            &[
                EmailAddress::new(None::<String>, "ada@work.example"),
                EmailAddress::new(None::<String>, "ada@home.example"),
            ],
        )
        .await
        .expect("the user names ada");
    Mail {
        _database: database,
        connection,
        inbox,
        account,
        ada,
    }
}

/// Sender name by address, as the message list draws it.
async fn list_names(mail: &Mail) -> Vec<(String, Option<String>)> {
    let mut rows: Vec<(String, Option<String>)> = MessageRepository::new(&mail.connection)
        .page(&ListQuery {
            scope: ListScope::Mailbox(mail.inbox),
            limit: 10,
            after: None,
        })
        .await
        .expect("a page")
        .into_iter()
        .map(|row| {
            let from = row.from.expect("a sender");
            (from.address, from.name)
        })
        .collect();
    rows.sort();
    rows
}

#[tokio::test]
async fn the_list_shows_the_users_name_for_every_one_of_their_addresses() {
    let mail = mail().await;
    assert_eq!(
        list_names(&mail).await,
        [
            ("ada@home.example".into(), Some("Countess of Lovelace".into())),
            ("ada@work.example".into(), Some("Countess of Lovelace".into())),
            (
                "not-ada@example.net".into(),
                Some("Ada Lovelace".into()),
            ),
        ],
        "both of ada's addresses carry the name the user gave her; the \
         stranger claiming her name keeps his header, and nothing else"
    );
}

#[tokio::test]
async fn a_conversations_participants_carry_the_users_name() {
    let mail = mail().await;
    let rows = ThreadRepository::new(&mail.connection)
        .page(&ThreadListQuery::in_mailbox(mail.account.id, mail.inbox).limit(10))
        .await
        .expect("threads");
    let engines = rows
        .iter()
        .find(|row| row.subject.as_deref() == Some("Engines"))
        .expect("the engines conversation");
    let names: Vec<Option<&str>> = engines
        .participants
        .iter()
        .map(|p| p.name.as_deref())
        .collect();
    assert_eq!(
        names,
        [Some("Countess of Lovelace"), Some("Countess of Lovelace")]
    );
}

#[tokio::test]
async fn the_reader_can_ask_for_the_users_names_by_address() {
    let mail = mail().await;
    let names = ContactRepository::new(&mail.connection)
        .user_names(&[
            "ADA@work.example".to_owned(),
            "not-ada@example.net".to_owned(),
        ])
        .await
        .expect("names");
    assert_eq!(
        names.get("ada@work.example").map(String::as_str),
        Some("Countess of Lovelace"),
        "matched case-insensitively, keyed by the normalised address"
    );
    assert!(
        !names.contains_key("not-ada@example.net"),
        "nobody owns the stranger's address, so there is no name to lend"
    );

    // The message itself stays what the mail said: it is written back by
    // other paths, and the header is the record.
    let stored = MessageRepository::new(&mail.connection)
        .page(&ListQuery {
            scope: ListScope::Mailbox(mail.inbox),
            limit: 10,
            after: None,
        })
        .await
        .expect("page")
        .into_iter()
        .find(|row| row.from.as_ref().is_some_and(|f| f.address == "ada@work.example"))
        .expect("ada's work mail");
    let message = MessageRepository::new(&mail.connection)
        .get(stored.id)
        .await
        .expect("get")
        .expect("the message");
    assert_eq!(message.from[0].name.as_deref(), Some("A. L."));
}

#[tokio::test]
async fn a_deleted_person_lends_their_name_to_nothing() {
    let mail = mail().await;
    postio_storage::sql::execute(
        &mail.connection,
        "UPDATE contacts SET state = 'deleted' WHERE id = ?1",
        [mail.ada.get()],
    )
    .await
    .expect("delete ada");
    let names = list_names(&mail).await;
    assert_eq!(
        names[0],
        ("ada@home.example".into(), Some("ada".into())),
        "back to what the mail said"
    );
    assert!(
        ContactRepository::new(&mail.connection)
            .user_names(&["ada@home.example".to_owned()])
            .await
            .expect("names")
            .is_empty()
    );
}
