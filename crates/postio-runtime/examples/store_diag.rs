use std::time::{Duration, Instant};

use chrono::Utc;
use postio_model::{AccountId, AccountScope};
use postio_search::facets::Scope;
use postio_search::results::ResultOrder;
use postio_storage::key::{Purpose, StoreKey};
use postio_storage::{Checkout, Store};

use postio_index::executor::{SearchRequest, search};

fn shape(sql: &str) -> String {
    let flat = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.len() > 110 {
        format!("{}…", &flat[..110])
    } else {
        flat
    }
}

/// Point this at a copy: the engine has no read-only open, and its own recovery on open can write.
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

    println!("\n== D: the plans the two halves of a match take ==");
    for sql in [
        "SELECT content_id FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, 'invoice')",
        "SELECT content_id FROM message_search_bodies WHERE fts_match(body_search, 'invoice')",
        "SELECT id FROM messages m
          WHERE m.deleted_locally = 0 AND m.account_id = 1
          ORDER BY m.received_at DESC, m.id DESC LIMIT 50",
    ] {
        explain(&connection, sql).await;
    }
}
