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

use rusqlite::Connection;

use postio_storage::migrate;

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

fn migrated() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory sqlite");
    connection
        .pragma_update(None, "foreign_keys", false)
        .expect("foreign keys off: this fills messages without their parents");
    migrate(&mut connection).expect("migrate");
    connection
}

fn plan(connection: &Connection, query: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .expect("a query plan");
    statement
        .query_map([], |row| row.get::<_, String>(3))
        .expect("plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("plan rows")
        .join("\n")
}

/// A mailbox the size of a real one, with three messages snoozed in it.
fn fill(connection: &Connection) {
    connection
        .execute_batch(&format!(
            "INSERT INTO messages (account_id, mailbox_id, remote_id, received_at, snoozed_until)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {MESSAGES})
             SELECT 1, 1 + (i % 4), 'r' || i, 1700000000 + i,
                    CASE WHEN i % 7000 = 0 THEN 1700000100 ELSE NULL END
               FROM n;"
        ))
        .expect("fill the mailbox");
    connection
        .execute_batch("ANALYZE")
        .expect("let the planner see what it is choosing between");
}

#[test]
fn waking_due_snoozes_seeks_the_snoozed_rows_instead_of_walking_the_account() {
    let connection = migrated();
    fill(&connection);

    let due = plan(&connection, DUE);
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

    let clear = plan(&connection, CLEAR);
    assert!(
        clear.contains("idx_messages_snoozed_due"),
        "the update that follows it walks the same rows and needs the same \
         index. It plans as:\n{clear}"
    );
}

#[test]
fn the_index_holds_only_snoozed_rows() {
    let connection = migrated();
    let definition: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            ["idx_messages_snoozed_due"],
            |row| row.get(0),
        )
        .expect("idx_messages_snoozed_due exists");

    // Partial, and that is the whole point: an index over every message would
    // be as large as the table and would have to be maintained on every
    // insert. This one holds the handful of rows that are actually snoozed.
    assert!(
        definition.contains("WHERE snoozed_until IS NOT NULL"),
        "the index must be partial, or it costs as much to keep as it saves: {definition}"
    );
}

#[test]
fn the_index_does_not_change_which_messages_wake() {
    let connection = migrated();
    fill(&connection);

    // Three of the 20,000 are snoozed and due (i = 7000, 14000, 21000 -- the
    // last is past the end), across whichever mailboxes the modulus put them
    // in. An index the planner declines to use would still return these, by
    // scanning, which is why the plan is asserted above as well.
    let mut statement = connection.prepare(DUE).expect("prepare");
    let woken: Vec<i64> = statement
        .query_map([], |row| row.get(0))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("rows");

    let expected: Vec<i64> = connection
        .prepare(
            "SELECT DISTINCT mailbox_id FROM messages
              WHERE snoozed_until IS NOT NULL AND snoozed_until <= 1700000500
              ORDER BY mailbox_id",
        )
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("rows")
        .collect::<Result<_, _>>()
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
