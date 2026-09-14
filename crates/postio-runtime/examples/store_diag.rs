//! Read-only search diagnosis against a real store.
//!
//! `postio-y47`'s bench corpus proved every search fast; a real store still
//! reported multi-second ones (#746). No bench could tell the two apart — the
//! gap was in what the corpus left empty, not in what it measured — so this
//! opens an existing store **read-only** and runs the real
//! `postio_index::search` over it, timed, then repeats the same search on a
//! warm connection to separate "the query is expensive" from "the cache
//! cannot hold the working set and every run re-decrypts it".
//!
//! Prints statement shapes, counts and durations only — never message
//! content, never a query a user typed, never the key.
//!
//! ```sh
//! POSTIO_DIAG_KEY=$(secret-tool lookup application postio account "local store encryption key") \
//!   cargo run --release -p postio-runtime --example store_diag [-- /path/to/postio.db]
//! ```
//!
//! `--release` is not optional: the page cipher is compiled at the profile's
//! opt level, so a debug build measures unoptimised crypto.
//!
//! # What this can no longer see
//!
//! It used to install SQLite's `trace_v2` profile hook and report a duration
//! **per statement** inside one search. This engine has no trace hook, so
//! sections A and D — the per-statement breakdown and the plans for the three
//! slowest — are gone with it. What is left is the whole-search timing, which
//! is what says *whether* there is a problem, and `EXPLAIN QUERY PLAN` over
//! the executor's own SQL, which is what usually says where.
//!
//! # Two traps #746 walked into before it found the real cost
//!
//! 1. **A `count(*)` wrapper un-measures scalar subqueries.** Timing a
//!    suspect statement by wrapping it in `SELECT count(*) FROM (...)`
//!    "proves" it fast, because the planner prunes subquery columns nothing
//!    reads — the expensive expressions never execute. To time a statement,
//!    run the statement: step every row and read every column, the way
//!    [`run_search`] does.
//! 2. **A plan line names an index without naming the key columns actually
//!    used.** `SEARCH ... USING INDEX idx_name (account_id=?)` can be true
//!    and still hide that the column that actually narrows the scan — say, an
//!    address — silently dropped out of the probe because it was compared
//!    against a *correlated subquery*, which cannot be used as an index key.
//!    Read the probe's parenthesised columns, not just which index got
//!    mentioned.

use std::time::{Duration, Instant};

use chrono::Utc;
use postio_model::{AccountId, AccountScope};
use postio_search::facets::Scope;
use postio_search::results::ResultOrder;
use postio_storage::key::{Purpose, StoreKey};
use postio_storage::{Checkout, Store};

use postio_index::executor::{SearchRequest, search};

/// One line of SQL, whitespace collapsed, cut to something scannable.
fn shape(sql: &str) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() > 110 {
        format!("{}…", &flat[..110])
    } else {
        flat
    }
}

/// Open the store at `path` under the master key in `key_hex`.
///
/// Not read-only in the file-permission sense any more — the engine has no
/// `SQLITE_OPEN_READ_ONLY` equivalent in its Rust API — so **point this at a
/// copy**, which is what the doc comment has always asked for. Nothing below
/// writes; the risk is the engine's own recovery on open.
async fn open(path: &str, key_hex: &str, cache_kib: i64) -> (Store, Checkout) {
    let master = StoreKey::from_hex(key_hex.trim()).expect("POSTIO_DIAG_KEY is not a key");
    let key = master.derive(Purpose::Database);
    let store = Store::open(path, &key).await.expect("open the store");
    let connection = store.connect().await.expect("a connection");
    connection
        .execute(&format!("PRAGMA cache_size = -{cache_kib}"), ())
        .await
        .expect("cache size");
    (store, connection)
}

/// Runs one search and prints the executor's answer and what it cost.
async fn run_search(connection: &Checkout, text: &str, order: ResultOrder) -> Duration {
    let query = postio_search::parse(text, Utc::now().date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(AccountId::new(1)),
        query: &query,
        scope: Scope::AllMail,
        limit: 200,
        order,
    };
    let start = Instant::now();
    let results = search(connection, &request, Utc::now())
        .await
        .expect("search");
    let total = start.elapsed();
    println!(
        "  '{text}' [{order:?}]: {} hits of {} total in {:?}",
        results.hits.len(),
        results.total_hits,
        total
    );
    total
}

async fn explain(connection: &Checkout, sql: &str) {
    println!("  EXPLAIN QUERY PLAN {}", shape(sql));
    let plan = postio_storage::sql::all(
        connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        (),
        |row| postio_storage::sql::RowExt::col::<String>(row, 3),
    )
    .await;
    match plan {
        Ok(steps) => {
            for step in steps {
                println!("      {step}");
            }
        }
        Err(error) => println!("      ({error})"),
    }
}

#[tokio::main]
async fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/.local/share/postio/postio.db",
            std::env::var("HOME").expect("HOME")
        )
    });
    let key_hex = std::env::var("POSTIO_DIAG_KEY")
        .expect("set POSTIO_DIAG_KEY to the master key hex (see the doc comment)");

    println!("== store shape (cold connection, app-sized 16 MiB cache) ==");
    let (_store, connection) = open(&path, &key_hex, 16_000).await;
    for (label, sql) in [
        ("messages", "SELECT count(*) FROM messages"),
        ("recipients", "SELECT count(*) FROM recipients"),
        ("contacts", "SELECT count(*) FROM contacts"),
        ("search documents", "SELECT count(*) FROM search_documents"),
        (
            "indexed body text (reads every overflow page: decrypt throughput)",
            "SELECT count(*), coalesce(sum(length(body_search)),0) FROM message_search_bodies",
        ),
    ] {
        let start = Instant::now();
        let row = postio_storage::sql::first(&connection, sql, (), |row| {
            Ok((
                postio_storage::sql::RowExt::col::<i64>(row, 0)?,
                postio_storage::sql::RowExt::col::<i64>(row, 1).ok(),
            ))
        })
        .await;
        match row {
            Ok(Some((count, bytes))) => {
                let bytes = bytes.map(|b| format!(", {:.1} MiB", b as f64 / (1024.0 * 1024.0)));
                println!(
                    "  {label}: {count}{} in {:?}",
                    bytes.unwrap_or_default(),
                    start.elapsed()
                );
            }
            Ok(None) => println!("  {label}: (no row)"),
            Err(error) => println!("  {label}: ({error})"),
        }
    }

    let terms = ["zzzqqxv", "invoice", "meeting", "unsubscribe", "the"];

    println!("\n== A: cold-ish searches, 16 MiB cache ==");
    for term in terms {
        run_search(&connection, term, ResultOrder::Relevance).await;
    }

    println!("\n== B: the same search three times on the same warm connection ==");
    for _ in 0..3 {
        run_search(&connection, "invoice", ResultOrder::Relevance).await;
    }
    println!("  -- and date order rather than relevance --");
    run_search(&connection, "invoice", ResultOrder::Newest).await;

    println!("\n== C: fresh connection, 256 MiB cache ==");
    let (_big_store, big) = open(&path, &key_hex, 256_000).await;
    for _ in 0..3 {
        run_search(&big, "invoice", ResultOrder::Relevance).await;
    }
    run_search(&big, "meeting", ResultOrder::Relevance).await;
    run_search(&big, "the", ResultOrder::Relevance).await;

    // The executor builds its SQL privately, so what can be explained from
    // out here is the shape rather than the statement. Both halves of the
    // union, which is where a search's cost lives.
    println!("\n== D: the plans the two halves of a match take ==");
    for sql in [
        "SELECT message_id FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, 'invoice')",
        "SELECT message_id FROM message_search_bodies WHERE fts_match(body_search, 'invoice')",
        "SELECT id FROM messages m
          WHERE m.deleted_locally = 0 AND m.account_id = 1
          ORDER BY m.received_at DESC, m.id DESC LIMIT 50",
    ] {
        explain(&connection, sql).await;
    }
}
