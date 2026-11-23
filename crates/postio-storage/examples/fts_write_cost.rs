//! Header inserts into `messages` must cost the same with and without the body index, which lives
//! on `message_search_bodies`.
//!
//! Fixtures only: this writes.

#![allow(clippy::disallowed_methods)] // the crate's own code prepares through `sql::statement`; a test may reach the engine directly

use postio_storage::{Store, test_support};
use std::time::Instant;

/// Reports the worst insert as well as the mean: a segment merge lands on one commit and a mean
/// hides it.
async fn header_batch(store: &Store, from: i64, rows: i64) -> (f64, f64) {
    let connection = store.connect().await.expect("checkout");
    let mut worst = 0.0f64;
    let mut total = 0.0f64;
    for n in from..from + rows {
        let one = Instant::now();
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, subject,
                                       remote_id)
                 SELECT account_id, id, ?2, ?2, ?3, ?4 FROM mailboxes WHERE id = 1",
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
            if round > 0 {
                for n in 0..300 {
                    let _ = connection
                        .execute(
                            "INSERT INTO message_search_bodies (content_id, body_search)
                             VALUES (?1, ?2)
                             ON CONFLICT (content_id) DO UPDATE
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
