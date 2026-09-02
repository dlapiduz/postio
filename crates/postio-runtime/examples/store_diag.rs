//! Read-only search diagnosis against a real store.
//!
//! `postio-y47`'s bench corpus proved every search fast; a real store still
//! reported multi-second ones (#746). No bench could tell the two apart —
//! the gap was in what the corpus left empty, not in what it measured — so
//! this opens an existing SQLCipher store **read-only** and runs the real
//! `postio_index::search` over it with a rusqlite `trace` profile hook, so
//! every SQL statement inside one search reports its own duration. Then
//! repeats the same search on a warm connection, and again under a larger
//! page cache, to separate "the query is expensive" from "the cache cannot
//! hold the working set and every run re-decrypts it".
//!
//! Prints statement shapes, counts and durations only — never message
//! content, never a query a user typed, never the key.
//!
//! ```sh
//! POSTIO_DIAG_KEY=$(secret-tool lookup application postio account "local store encryption key") \
//!   cargo run --release -p postio-runtime --example store_diag [-- /path/to/postio.db]
//! ```
//!
//! `--release` is not optional: the bundled SQLCipher is compiled at the
//! profile's opt level, so a debug build measures unoptimised crypto.
//!
//! # Two traps #746 walked into before it found the real cost
//!
//! 1. **A `count(*)` wrapper un-measures scalar subqueries.** Timing a
//!    suspect statement by wrapping it in `SELECT count(*) FROM (...)`
//!    "proves" it fast, because SQLite prunes subquery columns nothing
//!    reads — the expensive expressions never execute. To time a
//!    statement, run the statement: step every row and read every column,
//!    the way [`run_search`] and the plan section below both do.
//! 2. **A plan line names an index without naming the key columns actually
//!    used.** `EXPLAIN QUERY PLAN`'s `SEARCH ... USING INDEX idx_name
//!    (account_id=?)` can be true and still hide that the column that
//!    actually narrows the scan — say, an address — silently dropped out
//!    of the probe because it was compared against a *correlated
//!    subquery*, which SQLite cannot use as an index key. Read the probe's
//!    parenthesised columns, not just which index got mentioned.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use postio_model::{AccountId, AccountScope};
use postio_search::facets::Scope;
use postio_search::results::ResultOrder;
use postio_storage::key::{Purpose, StoreKey};
use rusqlite::trace::{TraceEvent, TraceEventCodes};
use rusqlite::{Connection, OpenFlags};

use postio_index::executor::{SearchRequest, search};

/// Statements the profile hook saw: (sql, duration).
static PROFILE: Mutex<Vec<(String, Duration)>> = Mutex::new(Vec::new());

/// `trace_v2`'s callback, filtered to the one event kind this cares about:
/// `profile` and `trace` are both deprecated in favor of this single hook.
fn record(event: TraceEvent<'_>) {
    if let TraceEvent::Profile(statement, took) = event {
        PROFILE
            .lock()
            .unwrap()
            .push((statement.sql().into_owned(), took));
    }
}

fn drain() -> Vec<(String, Duration)> {
    std::mem::take(&mut *PROFILE.lock().unwrap())
}

/// One line of SQL, whitespace collapsed, cut to something scannable.
fn shape(sql: &str) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() > 110 {
        format!("{}…", &flat[..110])
    } else {
        flat
    }
}

fn open(path: &str, key_hex: &str, cache_kib: i64) -> Connection {
    let master = StoreKey::from_hex(key_hex.trim()).expect("POSTIO_DIAG_KEY is not a key");
    let key = master.derive(Purpose::Database);
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("open the store read-only");
    connection
        .execute_batch("PRAGMA cipher_memory_security = OFF;")
        .expect("cipher_memory_security");
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", *key.to_hex()))
        .expect("PRAGMA key");
    connection
        .execute_batch(&format!(
            "PRAGMA cache_size = -{cache_kib}; PRAGMA busy_timeout = 5000; \
             PRAGMA temp_store = MEMORY;"
        ))
        .expect("pragmas");
    connection
}

/// Runs one search, prints the executor's answer and the profiled statements.
fn run_search(connection: &Connection, text: &str, order: ResultOrder, verbose: bool) -> Duration {
    let query = postio_search::parse(text, Utc::now().date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(AccountId::new(1)),
        query: &query,
        scope: Scope::AllMail,
        limit: 200,
        order,
    };
    drain();
    let start = Instant::now();
    let results = search(connection, &request, Utc::now()).expect("search");
    let total = start.elapsed();
    println!(
        "  '{text}' [{order:?}]: {} hits of {} total in {:?}",
        results.hits.len(),
        results.total_hits,
        total
    );
    if verbose {
        let mut statements = drain();
        statements.sort_by_key(|(_, took)| std::cmp::Reverse(*took));
        for (sql, took) in statements.iter().take(6) {
            println!("      {:>8.1?}  {}", took, shape(sql));
        }
    }
    total
}

fn explain(connection: &Connection, sql: &str) {
    println!("  EXPLAIN QUERY PLAN {}", shape(sql));
    let Ok(mut statement) = connection.prepare(&format!("EXPLAIN QUERY PLAN {sql}")) else {
        println!("      (did not prepare)");
        return;
    };
    // Unbound parameters read as NULL, which is fine for a plan.
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(3)?,
        ))
    }) else {
        println!("      (did not run)");
        return;
    };
    for row in rows.flatten() {
        println!("      [{} <- {}] {}", row.0, row.1, row.2);
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/.local/share/postio/postio.db",
            std::env::var("HOME").expect("HOME")
        )
    });
    let key_hex = std::env::var("POSTIO_DIAG_KEY")
        .expect("set POSTIO_DIAG_KEY to the master key hex (see the doc comment)");

    println!("== store shape (cold connection, app-sized 16 MiB cache) ==");
    let connection = open(&path, &key_hex, 16_000);
    for (label, sql) in [
        ("messages", "SELECT count(*) FROM messages"),
        ("recipients", "SELECT count(*) FROM recipients"),
        ("contacts", "SELECT count(*) FROM contacts"),
        (
            "messages_fts blocks",
            "SELECT count(*), coalesce(sum(length(block)),0) FROM messages_fts_data",
        ),
        (
            "bodies_fts blocks",
            "SELECT count(*), coalesce(sum(length(block)),0) FROM message_bodies_fts_data",
        ),
        (
            "bodies inline in messages (reads every overflow page: decrypt throughput)",
            "SELECT count(body_text), coalesce(sum(length(body_text)),0) FROM messages",
        ),
    ] {
        let start = Instant::now();
        let row: Result<(i64, Option<i64>), _> =
            connection.query_row(sql, [], |row| Ok((row.get(0)?, row.get(1).ok())));
        match row {
            Ok((count, bytes)) => {
                let bytes = bytes.map(|b| format!(", {:.1} MiB", b as f64 / (1024.0 * 1024.0)));
                println!(
                    "  {label}: {count}{} in {:?}",
                    bytes.unwrap_or_default(),
                    start.elapsed()
                );
            }
            Err(error) => println!("  {label}: ({error})"),
        }
    }

    let terms = ["zzzqqxv", "invoice", "meeting", "unsubscribe", "the"];

    println!("\n== A: cold-ish searches, 16 MiB cache, per-statement profile ==");
    connection.trace_v2(TraceEventCodes::SQLITE_TRACE_PROFILE, Some(record));
    for term in terms {
        run_search(&connection, term, ResultOrder::Relevance, true);
    }

    println!("\n== B: the same search three times on the same warm connection ==");
    for _ in 0..3 {
        run_search(&connection, "invoice", ResultOrder::Relevance, false);
    }
    println!("  -- and date order rather than relevance --");
    run_search(&connection, "invoice", ResultOrder::Newest, true);

    println!("\n== C: fresh connection, 256 MiB cache ==");
    let big = open(&path, &key_hex, 256_000);
    big.trace_v2(TraceEventCodes::SQLITE_TRACE_PROFILE, Some(record));
    for _ in 0..3 {
        run_search(&big, "invoice", ResultOrder::Relevance, false);
    }
    run_search(&big, "meeting", ResultOrder::Relevance, false);
    run_search(&big, "the", ResultOrder::Relevance, false);

    println!("\n== D: query plans for the slowest statements of a 16 MiB search ==");
    drain();
    run_search(&connection, "invoice", ResultOrder::Relevance, false);
    let mut statements = drain();
    statements.sort_by_key(|(_, took)| std::cmp::Reverse(*took));
    statements.truncate(3);
    for (sql, took) in &statements {
        println!("  ({took:?})");
        explain(&connection, sql);
    }
}
