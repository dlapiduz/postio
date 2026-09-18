//! What one pass of the body indexer costs as the index fills.
//!
//! The question this answers: `index_local_bodies` takes batches of
//! [`INDEX_BODY_BATCH`] until nothing is left, and each batch asks
//! [`messages_missing_body_text`] for the next candidates —
//!
//! ```sql
//! SELECT m.id FROM messages m
//!  WHERE m.body_state IN ('full', 'partial')
//!    AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
//!                     WHERE b.message_id = m.id)
//!  ORDER BY m.received_at DESC
//!  LIMIT ?1
//! ```
//!
//! Is that query's cost **constant** in how much has already been indexed, or
//! does it grow? It walks `idx_messages_recency` newest-first and the index
//! fills newest-first, so every pass has to walk past everything the previous
//! passes did before it reaches a candidate. If the cost per batch rises with
//! the number already indexed, the whole catch-up is quadratic in the size of
//! the mailbox, and a large archive spends hours where it should spend
//! minutes.
//!
//! Two things make that worse than a bare index walk, and both are visible in
//! the plan this prints:
//!
//! * `body_state` is **not** in `idx_messages_recency`
//!   (`received_at DESC, id DESC, deleted_locally, snoozed_until`), so every
//!   row the walk passes has to be fetched from `messages` to test it — and
//!   `messages` holds `body_text`/`body_html` inline, which is most of the
//!   table by bytes.
//! * the `NOT EXISTS` probe is a rowid seek into `message_search_bodies`, one
//!   per row walked.
//!
//! Synthetic mail only — `postio_storage::seed` writes `postio-model`'s
//! invented people. Nothing here reads a real store, so it is safe to run and
//! safe to paste the numbers anywhere.
//!
//! ```sh
//! cargo run --release -p postio-runtime --example body_index_cost -- \
//!     /tmp/postio-bodycost.db 20000
//! ```
//!
//! `--release` is not optional: the page cipher compiles at the profile's opt
//! level, so a debug build measures unoptimised crypto rather than the query.
//!
//! [`INDEX_BODY_BATCH`]: postio_session
//! [`messages_missing_body_text`]: postio_index::index::messages_missing_body_text

use std::path::PathBuf;
use std::time::{Duration, Instant};

use postio_storage::key::{Purpose, StoreKey};
use postio_storage::{Checkout, Store};

/// What the indexer asks for at a time (`postio_session::INDEX_BODY_BATCH`).
const BATCH: u32 = 200;

/// Roughly how many bytes of body a message carries.
///
/// The point of widening the rows: a measurement over rows that are only
/// headers would be measuring a table that fits in cache, and the real one
/// does not — about nine tenths of `messages` is body bytes.
const BODY_BYTES: usize = 4096;

/// A throwaway key. This store holds invented mail and is deleted after.
const DEV_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("postio-bodycost.db"));
    let count: usize = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(20_000);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let key = StoreKey::from_hex(DEV_KEY).expect("the dev key");
    let database = Store::open(&path, &key.derive(Purpose::Database))
        .await
        .expect("open the store");

    println!("seeding {count} messages…");
    let started = Instant::now();
    postio_storage::seed::seed_large(&database, 7, count).await;
    let connection = database.connect().await.expect("a connection");

    // The seeder writes `body_state = NotFetched`, which is not a candidate.
    // Make every message look like one whose body is already on this machine,
    // which is exactly the population the catch-up pass exists for.
    let filler = "lorem ipsum dolor sit amet ".repeat(BODY_BYTES / 27);
    connection
        .execute(
            "UPDATE messages SET body_state = 'full', body_text = ?1",
            postio_storage::bind![filler.as_str()],
        )
        .await
        .expect("widen the rows");
    println!(
        "seeded and widened in {:.1}s",
        started.elapsed().as_secs_f64()
    );

    explain(&connection).await;

    println!();
    println!(
        "{:>6} {:>10} {:>12} {:>12}",
        "batch", "indexed", "candidates_ms", "insert_ms"
    );

    let mut indexed = 0usize;
    let mut first: Option<Duration> = None;
    let mut last = Duration::ZERO;
    loop {
        let started = Instant::now();
        let candidates = postio_index::index::messages_missing_body_text(&connection, BATCH)
            .await
            .expect("candidates");
        let asked = started.elapsed();
        if candidates.is_empty() {
            break;
        }

        // Stand in for the real write. The pass reads and decompresses a body
        // per message before this; that half is linear in the batch and is not
        // what this is asking about.
        let started = Instant::now();
        for id in &candidates {
            connection
                .execute(
                    "INSERT OR REPLACE INTO message_search_bodies (message_id, body_search)
                     VALUES (?1, ?2)",
                    postio_storage::bind![*id, "indexed"],
                )
                .await
                .expect("insert");
        }
        let wrote = started.elapsed();

        indexed += candidates.len();
        first.get_or_insert(asked);
        last = asked;

        let batch = indexed / BATCH as usize;
        // Every batch is noise; the shape is what matters.
        if batch <= 3 || batch.is_multiple_of(10) {
            println!(
                "{:>6} {:>10} {:>12.1} {:>12.1}",
                batch,
                indexed,
                asked.as_secs_f64() * 1000.0,
                wrote.as_secs_f64() * 1000.0,
            );
        }
    }

    println!();
    let first = first.unwrap_or_default();
    println!(
        "candidate query: {:.1}ms on the first batch, {:.1}ms on the last — {:.1}x",
        first.as_secs_f64() * 1000.0,
        last.as_secs_f64() * 1000.0,
        last.as_secs_f64() / first.as_secs_f64().max(f64::MIN_POSITIVE),
    );
    println!(
        "a flat line means the pass is linear in the mailbox; a rising one means \
         it is quadratic, and the last batch of a {count}-message archive pays \
         for every batch before it."
    );

    drop(connection);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
}

/// What the engine says it will do, which is half the answer on its own.
async fn explain(connection: &Checkout) {
    let sql = "SELECT m.id
                 FROM messages m
                WHERE m.body_state IN ('full', 'partial')
                  AND NOT EXISTS (SELECT 1 FROM message_search_bodies b
                                   WHERE b.message_id = m.id)
                ORDER BY m.received_at DESC
                LIMIT 200";
    println!("\nEXPLAIN QUERY PLAN:");
    match postio_storage::sql::all(
        connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        (),
        |row| postio_storage::sql::RowExt::col::<String>(row, 3),
    )
    .await
    {
        Ok(steps) => {
            for step in steps {
                println!("  {step}");
            }
        }
        Err(error) => println!("  ({error})"),
    }
}
