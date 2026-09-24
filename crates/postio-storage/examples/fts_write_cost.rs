//! What the body index costs the writes that are not about bodies.
//!
//! A live folder sync sat in `commit_batch` for eight minutes on a
//! twenty-five-message folder. Sampling the process put eighteen of eighteen
//! samples inside `tantivy`, under
//! `turso_core::index_method::fts` -> `IndexMerger::write`.
//!
//! What this measured, and what the fix did: `messages_body_fts` **was** an
//! index on `messages`, so every write to that table -- a header sync that
//! never touches a body included -- went through the tantivy index and paid
//! whatever merge the *body backfill's* accumulated segments had made due
//! (14.6 ms mean / 529 ms worst at 1,200 body writes, against 2.9 / 77 with
//! the index dropped). The index is on its own table
//! (`message_search_bodies`) now, so the `with_index` column below writes
//! bodies there and the header inserts into `messages` stay at the
//! index-dropped cost -- the two columns should now match.
//!
//! ```text
//! cargo run -p postio-storage --example fts_write_cost --features test-support
//! ```
//!
//! Fixtures only: this writes.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::{Store, test_support};
use std::time::Instant;

/// One batch of header-shaped inserts: no body, no `body_search`.
///
/// Answers the **worst** insert as well as the mean, because that is the
/// shape being looked for: a segment merge is paid by one commit, not spread
/// over the batch, and a mean hides exactly the stall a person feels.
async fn header_batch(store: &Store, from: i64, rows: i64) -> (f64, f64) {
    let connection = store.connect().await.expect("checkout");
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    for n in from..from + rows {
        let one = Instant::now();
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, subject, remote_id)
                 SELECT account_id, id, ?2, ?3, ?4 FROM mailboxes WHERE id = 1",
                postio_storage::sql::bind![1_i64, n, format!("subject {n}"), format!("99:{n}")],
            )
            .await
            .expect("a header insert");
        let ms = one.elapsed().as_secs_f64() * 1000.0;
        total += ms;
        worst = worst.max(ms);
    }
    (total / rows as f64, worst)
}

/// A body of roughly the size real mail carries: the live store averaged
/// 31 KiB across 1,799 of them.
fn body(n: i64) -> String {
    let mut text = String::with_capacity(8 * 1024);
    for word in 0..900 {
        text.push_str(&format!("message{n}word{word} lorem ipsum dolor sit amet "));
    }
    text
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    for with_index in [false, true] {
        let store = test_support::temp().await;
        postio_storage::seed::seed_large(&store, 7, 200).await;
        let connection = store.connect().await.expect("checkout");

        if !with_index {
            connection
                .execute("DROP INDEX IF EXISTS messages_body_fts", ())
                .await
                .expect("drop the body index");
        }

        println!(
            "\nmessages_body_fts: {}",
            if with_index { "present" } else { "dropped" }
        );
        println!(
            "{:>18}  {:>14}  {:>14}",
            "after N body writes", "mean insert", "worst insert"
        );

        let mut indexed = 0;
        for round in 0..5 {
            // Churn the body index the way the backfill does: one body at a
            // time, each its own statement.
            if round > 0 {
                for n in 0..300 {
                    let _ = connection
                        .execute(
                            "INSERT INTO message_search_bodies (message_id, body_search)
                             VALUES (?1, ?2)
                             ON CONFLICT (message_id) DO UPDATE
                                SET body_search = excluded.body_search",
                            postio_storage::sql::bind![(n % 200) + 1, body(n + round * 1000)],
                        )
                        .await;
                    indexed += 1;
                }
            }
            let (mean, worst) = header_batch(&store, 1_000_000 + round * 1000, 40).await;
            println!("{indexed:>18}  {mean:>11.2} ms  {worst:>11.2} ms");
        }
    }
}
