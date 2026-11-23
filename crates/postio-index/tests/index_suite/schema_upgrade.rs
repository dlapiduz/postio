use postio_index::index::ensure_schema;
use postio_model::{AccountScope, Message};
use postio_search::facets::Scope;
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

const OLD_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS search_documents (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    sender      TEXT NOT NULL DEFAULT '',
    recipients  TEXT NOT NULL DEFAULT '',
    subject     TEXT NOT NULL DEFAULT '',
    filenames   TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS search_documents_fts ON search_documents
    USING fts (sender, recipients, subject, filenames);
";

async fn a_listed_message(connection: &Connection, subject: &str) -> Message {
    let (account, mailbox) = test_support::account_with_inbox(connection).await;
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_string());
    message.list_id = Some("harbour-dev.lists.example.org".to_string());
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create message");
    message
}

async fn hits(connection: &Connection, account: postio_model::AccountId, query: &str) -> Vec<i64> {
    let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
    postio_index::search(
        connection,
        &postio_index::SearchRequest {
            account: AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 10,
            order: postio_search::ResultOrder::Relevance,
        },
        chrono::Utc::now(),
    )
    .await
    .expect("search runs")
    .hits
    .iter()
    .map(|hit| hit.message_id.get())
    .collect()
}

#[tokio::test]
async fn a_store_from_before_list_id_gains_the_column_and_searches() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    connection
        .execute_batch(OLD_SCHEMA)
        .await
        .expect("the old index schema applies");

    ensure_schema(&connection)
        .await
        .expect("today's schema applies over the old one");

    let message = a_listed_message(&connection, "Tuesday walkthrough").await;

    assert_eq!(
        hits(&connection, message.account_id, "list:harbour-dev").await,
        vec![message.id.get()],
        "the upgraded index answers the query the new column exists for"
    );
}

#[tokio::test]
async fn a_store_already_broken_by_the_mismatch_recovers() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    connection
        .execute_batch(OLD_SCHEMA)
        .await
        .expect("the old index schema applies");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS search_schema (
                 half TEXT PRIMARY KEY, version INTEGER NOT NULL);
             INSERT INTO search_schema (half, version) VALUES ('metadata', 99)
                 ON CONFLICT (half) DO UPDATE SET version = 99;",
        )
        .await
        .expect("the mismatched version applies");

    connection
        .execute("DELETE FROM search_schema WHERE half = 'metadata'", ())
        .await
        .expect("the claim is cleared");

    ensure_schema(&connection)
        .await
        .expect("the repaired schema applies");

    let message = a_listed_message(&connection, "Wednesday walkthrough").await;
    assert_eq!(
        hits(&connection, message.account_id, "walkthrough").await,
        vec![message.id.get()],
        "a store the mismatch had already broken searches again"
    );
}

#[tokio::test]
async fn a_metadata_upgrade_never_drops_the_body_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let message = a_listed_message(&connection, "With a body").await;
    postio_index::index::index_body(&connection, message.id.get(), Some("the difference engine"))
        .await
        .expect("index a body");

    connection
        .execute(
            "UPDATE search_schema SET version = version - 1 WHERE half = 'metadata'",
            (),
        )
        .await
        .expect("the version regresses");

    ensure_schema(&connection)
        .await
        .expect("the rebuild applies");

    assert_eq!(
        hits(&connection, message.account_id, "difference").await,
        vec![message.id.get()],
        "the body index survived the metadata rebuild untouched"
    );
    assert_eq!(
        hits(&connection, message.account_id, "list:harbour-dev").await,
        vec![message.id.get()],
        "and the rebuilt metadata half still indexes what the store holds"
    );
}
