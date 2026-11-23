//! Times one small insert against a full-text index at the live store's scale: tantivy merges
//! inside whichever commit comes next.
//!
//! Fixtures only: this writes.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::test_support;
use std::time::Instant;

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
                "INSERT INTO message_search_bodies (content_id, body_search)
                 VALUES (?1, ?2)
                 ON CONFLICT (content_id) DO UPDATE SET body_search = excluded.body_search",
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

    println!("\ntiming 20 single-row header inserts against that index:");
    let mut worst = 0.0f64;
    for n in 0..20 {
        let one = Instant::now();
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, subject,
                                       remote_id)
                 SELECT account_id, id, ?2, ?2, ?3, ?4 FROM mailboxes WHERE id = 1",
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
