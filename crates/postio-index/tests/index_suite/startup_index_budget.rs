use chrono::{TimeZone, Utc};
use postio_index::index::ensure_schema;
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, install};

const MESSAGES: usize = 2_000;

#[tokio::test]
async fn an_ordinary_start_does_not_reindex_the_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let messages = MessageRepository::new(&connection);
    for nth in 0..MESSAGES {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            + chrono::Duration::seconds(nth as i64);
        let mut message = Message::new(account.id, mailbox, received);
        message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly report {nth}"));
        messages.create(&mut message).await.expect("create message");
    }

    install(&connection);

    let building =
        counted_async(async || ensure_schema(&connection).await.expect("the first start")).await;

    let ordinary =
        counted_async(async || ensure_schema(&connection).await.expect("an ordinary start")).await;

    assert!(
        building.statements > CEILING,
        "the start that builds the index ran only {} statements, at or under \
         the ceiling the ordinary start is held to -- so the two are \
         indistinguishable and the assertion below means nothing",
        building.statements
    );

    assert!(
        ordinary.statements <= CEILING,
        "an ordinary start ran {} statements over a {MESSAGES}-message store, \
         against {} for the start that built the index. Startup work that \
         scales with the mailbox is what §18's 500ms budget cannot survive, \
         and it is invisible in behaviour -- search returns the same results \
         either way.",
        ordinary.statements,
        building.statements
    );
}

const CEILING: usize = 8;

#[tokio::test]
async fn an_ordinary_start_does_not_re_execute_the_schema() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");

    ensure_schema(&connection).await.expect("the first start");

    install(&connection);
    let ordinary =
        counted_async(async || ensure_schema(&connection).await.expect("an ordinary start")).await;

    assert!(
        ordinary.statements <= CEILING,
        "a start with every schema half already current ran {} statements,          over a ceiling of {CEILING}. The `SCHEMA` batch is being re-executed          when there is nothing to create; on a populated store one of its          no-op `CREATE TRIGGER IF NOT EXISTS` statements costs more than          everything else in opening the store put together (#1113).",
        ordinary.statements
    );
}
