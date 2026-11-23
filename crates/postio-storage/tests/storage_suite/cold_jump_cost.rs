//! Asserts the cause of #638: the columns every list query filters on (`deleted_locally`,
//! `snoozed_until`) are in the list indexes, so an `OFFSET` skips inside the index. Counts and VDBE
//! steps cannot see it, and wall-clock is too flaky.

use postio_model::MailboxRole;
use postio_storage::Connection;
use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::test_support;

const LIST_INDEXES: &[&str] = &[
    "idx_messages_list",
    "idx_messages_account_list",
    "idx_messages_recency",
    "idx_messages_thread",
];

const FILTER_COLUMNS: &[&str] = &["deleted_locally", "snoozed_until"];

async fn columns_of(connection: &Connection, index: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA index_info({index})"))
        .await
        .expect("the index exists");
    postio_storage::sql::mapped(&mut statement, (), |row| {
        postio_storage::sql::RowExt::col::<Option<String>>(row, 2)
    })
    .await
    .expect("its columns")
    .into_iter()
    .flatten()
    .collect()
}

async fn missing_from(connection: &Connection, index: &str) -> Vec<&'static str> {
    let columns = columns_of(connection, index).await;
    FILTER_COLUMNS
        .iter()
        .copied()
        .filter(|filter| !columns.iter().any(|column| column == filter))
        .collect()
}

#[tokio::test]
async fn a_narrow_list_index_is_reported_as_missing_its_filters() {
    // The control: the same check must fail on the shape migration 0005 replaced, built here rather
    // than by undoing the migration.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    connection
        .execute_batch(
            "CREATE INDEX idx_probe_narrow_list
                 ON messages (mailbox_id, received_at DESC, id DESC);",
        )
        .await
        .expect("the pre-0005 shape");

    assert_eq!(
        missing_from(&connection, "idx_probe_narrow_list").await,
        FILTER_COLUMNS,
        "the check does not notice an index that carries neither filter \
         column, so it cannot be what stands between this schema and #638"
    );
    assert!(
        missing_from(&connection, "idx_messages_list")
            .await
            .is_empty(),
        "and it must not report the migrated index, or it would fail for \
         every schema alike and mean nothing"
    );
}

#[tokio::test]
async fn every_list_index_carries_the_columns_every_list_query_filters_on() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");

    for index in LIST_INDEXES {
        let columns = columns_of(&connection, index).await;

        assert!(
            !columns.is_empty(),
            "{index} is not in the schema at all; the list scopes name it"
        );
        for filter in FILTER_COLUMNS {
            assert!(
                columns.iter().any(|column| column == filter),
                "{index} does not carry `{filter}`, which every list query \
                 filters on. SQLite counts only rows that pass the WHERE \
                 toward an OFFSET, so it will fetch each skipped row from the \
                 table to test it: a jump halfway down a mailbox becomes one \
                 table read per row skipped, and under SQLCipher a decrypt \
                 with it. That is #638 — it measured 207ms against 3.5ms. \
                 The index holds {columns:?}."
            );
        }
    }
}

#[tokio::test]
async fn a_deep_page_returns_the_same_rows_the_narrow_index_would_have() {
    let database = test_support::temp().await;
    let report = seed_small(&database, 11).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let connection = database.connect().await.expect("a connection");
    let messages = MessageRepository::new(&connection);
    let query = ListQuery {
        scope: ListScope::Mailbox(inbox),
        limit: 5,
        after: None,
    };

    let through_the_index: Vec<_> = messages
        .page_at(&query, 3)
        .await
        .expect("a page three in")
        .iter()
        .map(|row| row.id)
        .collect();

    // The same window without the index: `+0` defeats its use without changing the query.
    let unindexed: Vec<_> = postio_storage::sql::all(
        &connection,
        "SELECT id FROM messages
          WHERE mailbox_id + 0 = ?1
            AND deleted_locally = 0
            AND (snoozed_until IS NULL
                 OR snoozed_until <= strftime('%s','now') * 1000)
          ORDER BY received_at DESC, id DESC
          LIMIT 5 OFFSET 3",
        [inbox.get()],
        |row| {
            Ok(postio_model::ids::MessageId::new(
                postio_storage::sql::RowExt::col::<i64>(row, 0)?,
            ))
        },
    )
    .await
    .expect("rows");

    assert_eq!(
        through_the_index, unindexed,
        "the widened index answers a deep page differently from a scan of the \
         same predicate, so it is not the same query any more"
    );
    assert!(
        !through_the_index.is_empty(),
        "the seed must leave enough messages for a page three in to mean \
         something"
    );
}
