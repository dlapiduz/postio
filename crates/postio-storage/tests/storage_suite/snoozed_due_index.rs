//! Waking due snoozes is served by an index over snoozed rows, not by walking
//! the mailbox (#1237).
//!
//! `Engine`'s poll tick calls `wake_due_snoozes` every five seconds for the
//! life of the process, and the query behind it asks a question whose answer
//! is almost always "none":
//!
//! ```text
//! SELECT DISTINCT mailbox_id FROM messages
//!  WHERE account_id = ?1 AND snoozed_until IS NOT NULL AND snoozed_until <= ?2
//! ```
//!
//! Before this, the planner served it from `idx_messages_account_list`, whose
//! leading column is `account_id` and whose `snoozed_until` sits fourth. That
//! seeks to the account and then walks **every message it has**, testing each
//! one. Nothing is returned and every entry is read.
//!
//! # Why an O(n) walk that returns nothing is not free here
//!
//! The store is SQLCipher. Every page read is an AES-CBC decrypt and an
//! HMAC-SHA512, so the walk is thousands of hashes to conclude there is
//! nothing to do — every five seconds, forever. A ring-buffer profile of the
//! running application (#1216) caught a burst 92.67% in `postio-sync` with
//! `sha512_block_data_order_avx2` at 24.80% of samples, over
//! `pcache1Fetch` and `sqlite3BtreeTableMoveto`.
//!
//! # Why this is a plan test and not a `counting` test
//!
//! `test_support::counting` reads statements, rows and trigger firings off
//! SQLite's trace hook. The old plan produces **zero rows** — it scans and
//! matches nothing — so a row count cannot tell the two plans apart. The
//! difference is in entries *visited*, which the trace hook does not report.
//! The plan is the thing that changed and the plan is what is asserted, the
//! same instrument `contact_rank_index.rs` uses for the same reason.
//!
//! Two things are asserted together, for the reason `draft_indexes.rs` gives:
//! an index the planner declines to use still returns the right rows, by
//! scanning, so neither the shape nor the results alone can fail usefully.

use postio_storage::Connection;


/// Enough mail that a walk over all of it is a real cost, and enough that
/// SQLite would not simply scan a tiny table whatever the index says.
const MESSAGES: usize = 20_000;

/// The query `MessageRepository::wake_due` runs, verbatim.
const DUE: &str = "SELECT DISTINCT mailbox_id FROM messages
     WHERE account_id = 1 AND snoozed_until IS NOT NULL AND snoozed_until <= 1700000500
     ORDER BY mailbox_id";

/// And the update it runs when that finds something.
const CLEAR: &str = "UPDATE messages SET snoozed_until = NULL
     WHERE account_id = 1 AND snoozed_until IS NOT NULL AND snoozed_until <= 1700000500";

async fn migrated() -> (postio_storage::Store, postio_storage::Checkout) {
    let store = postio_storage::test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    (store, connection)
}

async fn plan(connection: &Connection, query: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .await
        .expect("a query plan");
    postio_storage::sql::mapped(&mut statement, (), |row| postio_storage::sql::RowExt::col::<String>(row, 3))
        .await
        .expect("plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("plan rows")
        .join("\n")
}

/// A mailbox the size of a real one, with three messages snoozed in it.
async fn fill(connection: &Connection) {
    connection
        .execute_batch(&format!(
            "INSERT INTO messages (account_id, mailbox_id, remote_id, received_at, snoozed_until)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {MESSAGES})
             SELECT 1, 1 + (i % 4), 'r' || i, 1700000000 + i,
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
async fn the_index_holds_only_snoozed_rows() {
    let (_store, connection) = migrated().await;
    let definition: String = postio_storage::sql::one(
        &connection,
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
        ["idx_messages_snoozed_due"],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("idx_messages_snoozed_due exists");

    // Partial, and that is the whole point: an index over every message would
    // be as large as the table and would have to be maintained on every
    // insert. This one holds the handful of rows that are actually snoozed.
    assert!(
        definition.contains("WHERE snoozed_until IS NOT NULL"),
        "the index must be partial, or it costs as much to keep as it saves: {definition}"
    );
}

#[tokio::test]
async fn the_index_does_not_change_which_messages_wake() {
    let (_store, connection) = migrated().await;
    fill(&connection).await;

    // Three of the 20,000 are snoozed and due (i = 7000, 14000, 21000 -- the
    // last is past the end), across whichever mailboxes the modulus put them
    // in. An index the planner declines to use would still return these, by
    // scanning, which is why the plan is asserted above as well.
    let mut statement = connection.prepare(DUE).await.expect("prepare");
    let woken: Vec<i64> = postio_storage::sql::mapped(&mut statement, (), |row| postio_storage::sql::RowExt::col(row, 0))
        .await
        .expect("rows")
        .collect::<Result<_, _>>()
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
