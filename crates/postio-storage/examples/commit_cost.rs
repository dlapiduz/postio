//! What one small transaction costs, against store size.
//!
//! A live folder sync sat in `commit_batch` for eight minutes on a
//! twenty-five-message folder, and a stack sample put it inside
//! `Pager::commit_tx -> Pager::checkpoint -> DatabaseFile::sync` -- an
//! `fsync` on the *database*, from an ordinary commit.
//!
//! If commit cost is flat in store size, that sample was a coincidence. If it
//! climbs, then every transaction is paying to checkpoint the whole write-ahead
//! log, and a sync that commits in batches pays it once per batch -- which is
//! the difference between a mail client and a pathology.
//!
//! ```text
//! cargo run -p postio-storage --example commit_cost --features test-support
//! ```
//!
//! Fixtures only: this writes, so it must never be pointed at real mail.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::test_support;
use std::time::Instant;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("{:>12}  {:>12}  {:>14}", "rows", "file", "per commit");

    let store = test_support::temp().await;
    postio_storage::seed::seed_large(&store, 7, 200).await;

    for round in 0..6 {
        // Grow the store between measurements.
        if round > 0 {
            postio_storage::seed::seed_large(&store, 7 + round, 4_000).await;
        }
        let connection = store.connect().await.expect("checkout");
        let rows: i64 =
            postio_storage::sql::scalar(&connection, "SELECT count(*) FROM messages", ())
                .await
                .expect("count");
        let file = std::fs::metadata(store.directory().join("postio.db"))
            .map(|meta| meta.len())
            .unwrap_or(0);

        // Twenty of the smallest write there is, each its own transaction --
        // the shape `commit_batch` repeats.
        const COMMITS: u32 = 20;
        let start = Instant::now();
        for n in 0..COMMITS {
            connection
                .execute(
                    "UPDATE mailboxes SET total_count = ?1 WHERE id = 1",
                    (i64::from(n),),
                )
                .await
                .expect("a one-row write");
        }
        let each = start.elapsed().as_secs_f64() * 1000.0 / f64::from(COMMITS);

        println!("{rows:>12}  {:>12}  {each:>11.2} ms", bytes(file));
    }
}

fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
