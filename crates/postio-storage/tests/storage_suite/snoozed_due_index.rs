use postio_storage::Connection;

const MESSAGES: usize = 20_000;

/// The query `MessageRepository::wake_due` runs, verbatim. It has no `ORDER BY`, which would make
/// the planner decline this index for `idx_messages_list`; `wake_due` sorts in Rust, and this must
/// stay in step.
const DUE: &str = "SELECT DISTINCT mailbox_id FROM messages
     WHERE account_id = 1 AND snoozed_until IS NOT NULL AND snoozed_until <= 1700000500";

const CLEAR: &str = "UPDATE messages SET snoozed_until = NULL
     WHERE account_id = 1 AND snoozed_until IS NOT NULL AND snoozed_until <= 1700000500";

async fn migrated() -> (postio_storage::Store, postio_storage::Checkout) {
    let store = postio_storage::test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    (store, connection)
}

async fn plan(connection: &Connection, query: &str) -> String {
    postio_storage::test_support::plan(connection, query).await
}

async fn fill(connection: &Connection) {
    let account = postio_storage::test_support::account(connection).await;
    for index in 0..4 {
        postio_storage::test_support::mailbox(connection, &account, &format!("Box{index}")).await;
    }
    connection
        .execute_batch(&format!(
            "INSERT INTO messages (account_id, mailbox_id, remote_id, received_at, sort_at, snoozed_until)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {MESSAGES})
             SELECT 1, 1 + (i % 4), 'r' || i, 1700000000 + i, 1700000000 + i,
                    CASE WHEN i % 7000 = 0 THEN 1700000100 ELSE NULL END
               FROM n;"
        ))
        .await
        .expect("fill the mailbox");
    connection
        .execute_batch("ANALYZE")
        .await
        .expect("let the planner see what it is choosing between");
}

#[tokio::test]
async fn waking_due_snoozes_seeks_the_snoozed_rows_instead_of_walking_the_account() {
    let (_store, connection) = migrated().await;
    fill(&connection).await;

    let due = plan(&connection, DUE).await;
    assert!(
        due.contains("idx_messages_snoozed_due"),
        "the due-snooze query must be served by the partial index over snoozed \
         rows. It plans as:\n{due}"
    );
    assert!(
        !due.contains("idx_messages_account_list"),
        "and must not fall back to walking every message the account has, \
         which is what it did before #1237:\n{due}"
    );

    let clear = plan(&connection, CLEAR).await;
    assert!(
        clear.contains("idx_messages_snoozed_due"),
        "the update that follows it walks the same rows and needs the same \
         index. It plans as:\n{clear}"
    );
}

#[tokio::test]
async fn the_index_leads_with_the_column_that_selects() {
    let (_store, connection) = migrated().await;
    let definition: String = postio_storage::sql::one(
        &connection,
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
        ["idx_messages_snoozed_due"],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("idx_messages_snoozed_due exists");

    // The index is whole, not partial: this engine will not read through a partial index
    // (docs/notes/2026-09-12-a-partial-index-the-planner-will-not-read.md).
    assert!(
        !definition.to_ascii_uppercase().contains(" WHERE "),
        "the index is partial again, and this engine's planner will not read \
         through one -- so the poll tick is back to walking every message the \
         account has. See \
         docs/notes/2026-09-12-a-partial-index-the-planner-will-not-read.md:\n  {definition}"
    );
    assert!(
        definition.contains("(account_id, snoozed_until"),
        "the seek needs the account first and the snooze time second: {definition}"
    );
}

#[tokio::test]
async fn the_index_does_not_change_which_messages_wake() {
    let (_store, connection) = migrated().await;
    fill(&connection).await;

    let mut statement = connection.prepare(DUE).await.expect("prepare");
    let woken: Vec<i64> = postio_storage::sql::mapped(&mut statement, (), |row| {
        postio_storage::sql::RowExt::col(row, 0)
    })
    .await
    .expect("rows");

    let expected: Vec<i64> = postio_storage::sql::all(
        &connection,
        "SELECT DISTINCT mailbox_id FROM messages
          WHERE snoozed_until IS NOT NULL AND snoozed_until <= 1700000500
          ORDER BY mailbox_id",
        (),
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("rows");

    assert_eq!(
        woken, expected,
        "the same mailboxes wake, index or no index"
    );
    assert!(
        !woken.is_empty(),
        "the fixture must actually snooze something"
    );
}
