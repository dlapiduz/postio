use chrono::{TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::UidValidity;
use postio_storage::sql::{self, RowExt as _};
use postio_storage::test_support::{self, counting};
use postio_sync::sync_mailbox;

fn addressed(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         To: Bo <bo@example.org>, cy@example.net, Di <di@example.test>\r\n\
         Cc: Ed <ed@example.com>\r\n\
         Subject: Harbour schedule {n}\r\n\
         Message-ID: <harbour-{n}@example.com>\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         The tide table for week {n}.\r\n"
    )
    .into_bytes()
}

#[tokio::test]
async fn a_first_sync_writes_each_search_document_once() {
    let mut inbox = MockMailbox::new("INBOX").uid_validity(UidValidity::new(7));
    for n in 1..=12 {
        inbox = inbox.message(
            MockMessage::new(addressed(n))
                .with_internal_date(Utc.timestamp_opt(1_770_000_000 + i64::from(n), 0).unwrap()),
        );
    }
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = test_support::account(&connection).await;
    let mailbox = test_support::mailbox(&connection, &account, "INBOX").await;
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the search schema");
    sql::batch(
        &connection,
        "CREATE TABLE document_writes (id INTEGER PRIMARY KEY, content_id INTEGER);
         CREATE TRIGGER count_document_inserts AFTER INSERT ON search_documents
         BEGIN INSERT INTO document_writes (content_id) VALUES (new.content_id); END;
         CREATE TRIGGER count_document_updates AFTER UPDATE ON search_documents
         BEGIN INSERT INTO document_writes (content_id) VALUES (new.content_id); END;",
    )
    .await
    .expect("the counting triggers");

    counting::record();
    sync_mailbox(&connection, &backend, &mailbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");
    let issued = counting::recorded();

    // A plan walking `messages` to find the documents would be #1707's scan again; `scan_audit`
    // cannot see it (no search schema there).
    let mut scanned = Vec::new();
    for sql in issued.keys().filter(|sql| sql.contains("search_documents")) {
        let steps = counting::unbounded(&connection, sql, counting::GROWING_TABLES).await;
        if !steps.is_empty() {
            scanned.push(format!(
                "{steps:?}: {}",
                sql.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
    }
    assert!(
        issued
            .keys()
            .any(|sql| sql.contains("INSERT INTO search_documents")),
        "the sync never wrote the documents, so this checked nothing"
    );
    assert!(
        scanned.is_empty(),
        "indexing a unit reads a whole table: {scanned:#?}"
    );

    let writes: i64 = sql::one(
        &connection,
        "SELECT count(*) FROM document_writes",
        (),
        |row| row.col(0),
    )
    .await
    .expect("a count");
    assert_eq!(
        writes, 12,
        "twelve messages of six addresses each wrote their search documents {writes} times"
    );

    let found: i64 = sql::one(
        &connection,
        "SELECT count(*) FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, 'di')",
        (),
        |row| row.col(0),
    )
    .await
    .expect("a search");
    assert_eq!(found, 12, "every message is found by a cc'd address");
    let deferred: i64 = sql::one(
        &connection,
        "SELECT count(*) FROM search_documents_deferred",
        (),
        |row| row.col(0),
    )
    .await
    .expect("a count");
    assert_eq!(deferred, 0, "the sync left the triggers standing aside");
}
