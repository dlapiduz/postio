//! `sql::each`: a read that streams its rows to a visitor and can stop early.
//!
//! What a fold over a large match wants (spec 010's conversation search):
//! no `Vec` of every row, and no reading past the point where the visitor
//! has what it needs. Stopping drops the rows, so the connection is free for
//! the next statement, and the read is counted like every other.

use postio_storage::sql::{self, RowExt as _};
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

#[tokio::test]
async fn each_stops_when_the_visitor_says_so_and_frees_the_connection() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    connection
        .execute_batch(
            "CREATE TABLE numbers (n INTEGER PRIMARY KEY);
             INSERT INTO numbers (n) VALUES (1), (2), (3), (4), (5), (6), (7), (8);",
        )
        .await
        .expect("a table of numbers");

    let mut seen = Vec::new();
    let mut visited = 0;
    let counts = counted_async(async || {
        visited = sql::each(&connection, "SELECT n FROM numbers ORDER BY n", (), |row| {
            seen.push(row.col::<i64>(0)?);
            Ok(seen.len() < 3)
        })
        .await
        .expect("a streamed read");
    })
    .await;
    assert_eq!(seen, [1, 2, 3], "the visitor stopped it at the third row");
    assert_eq!(visited, 3);
    assert_eq!(counts.statements, 1);
    assert_eq!(counts.rows, 3, "rows visited, not rows the table holds");

    let total: i64 = sql::one(&connection, "SELECT count(*) FROM numbers", (), |row| {
        row.col(0)
    })
    .await
    .expect("the connection is free again");
    assert_eq!(total, 8);

    let mut all = 0;
    sql::each(&connection, "SELECT n FROM numbers", (), |_| {
        all += 1;
        Ok(true)
    })
    .await
    .expect("a read to the end");
    assert_eq!(all, 8, "a visitor that never stops sees every row");
}
