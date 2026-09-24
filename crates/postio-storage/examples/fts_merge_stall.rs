//! Does one small write pay for the whole full-text index?
//!
//! A live folder sync sat in `commit_batch` for **eight minutes** on a
//! twenty-five-message folder. Eighteen of eighteen stack samples were inside
//! `tantivy`, on the sync's own thread, under `turso_core::index_method::fts`
//! -> `IndexMerger::write` / `SegmentWriter::for_segment`.
//!
//! Twenty-five messages cannot be eight minutes of indexing. The suspicion is
//! that the *size of the index* is what was being paid for, not the size of
//! the write: tantivy merges segments, the merge runs inside whichever
//! transaction happens to commit next, and a sync that writes a handful of
//! rows can inherit a merge the body backfill's thousands of documents made
//! due.
//!
//! This builds an index at the live store's scale and then times one small
//! insert against it.
//!
//! ```text
//! cargo run --release -p postio-storage --example fts_merge_stall --features test-support
//! ```
//!
//! Fixtures only: this writes.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::test_support;
use std::time::Instant;

/// Roughly what the live store averaged per body: 31 KiB.
fn body(n: i64) -> String {
    let mut text = String::with_capacity(32 * 1024);
    for word in 0..3_600 {
        text.push_str(&format!("m{n}w{word} lorem ipsum dolor sit amet "));
    }
    text
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let messages: i64 = std::env::var("MESSAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4_000);
    let bodies: i64 = std::env::var("BODIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_500);

    let store = test_support::temp().await;
    println!("seeding {messages} messages...");
    postio_storage::seed::seed_large(&store, 7, messages as usize).await;
    let connection = store.connect().await.expect("checkout");

    println!("indexing {bodies} bodies of ~31 KiB, one statement each...");
    let start = Instant::now();
    for n in 1..=bodies {
        connection
            .execute(
                "INSERT INTO message_search_bodies (message_id, body_search)
                 VALUES (?1, ?2)
                 ON CONFLICT (message_id) DO UPDATE SET body_search = excluded.body_search",
                postio_storage::sql::bind![n, body(n)],
            )
            .await
            .expect("index a body");
        if n % 250 == 0 {
            println!(
                "  {n} bodies  ({:.1} s elapsed)",
                start.elapsed().as_secs_f64()
            );
        }
    }

    // Now the question: one row, nothing to do with bodies.
    println!("\ntiming 20 single-row header inserts against that index:");
    let mut worst = 0.0f64;
    for n in 0..20 {
        let one = Instant::now();
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, subject, remote_id)
                 SELECT account_id, id, ?2, ?3, ?4 FROM mailboxes WHERE id = 1",
                postio_storage::sql::bind![
                    1_i64,
                    9_000_000 + n,
                    format!("subject {n}"),
                    format!("77:{n}")
                ],
            )
            .await
            .expect("a header insert");
        let ms = one.elapsed().as_secs_f64() * 1000.0;
        worst = worst.max(ms);
        if ms > 100.0 {
            println!("  insert {n}: {ms:.0} ms   <-- paid for a merge");
        }
    }
    println!("worst single insert: {worst:.1} ms");
}
