//! What a real store holds, and what it costs — counts and bytes only.
//!
//! `docs/PERFORMANCE.md` says every wall-clock figure in it was taken against
//! SQLCipher and none has been re-measured. Two of them are expected to move
//! and neither can be taken from a fixture: **store size**, which should grow
//! (sixteen partial indexes lost their predicates, and message text is no
//! longer compressed), and how that size is distributed.
//!
//! ```text
//! POSTIO_STORE=~/scratch/postio-run/state/data/postio/postio.db \
//! POSTIO_STORE_KEY=$(secret-tool lookup application postio \
//!     account 'local store encryption key' xdg:schema org.postio.Account) \
//!   cargo run -p postio-storage --example store_census
//! ```
//!
//! **Point it at a copy, or at a store nothing is using.** This engine has no
//! read-only open (ADR 0038), so "read-only" here is a promise this file
//! keeps rather than one the engine enforces: it issues `SELECT` and `PRAGMA`
//! and nothing else.
//!
//! Ids, counts and outcomes only. No subject, address or body is read, and
//! none is printed — this runs against somebody's actual mail.

use postio_storage::Store;
use postio_storage::key::StoreKey;
use postio_storage::sql::RowExt;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let path = std::env::var("POSTIO_STORE").expect("POSTIO_STORE is the path to postio.db");
    let path = shellexpand(&path);
    let hex = std::env::var("POSTIO_STORE_KEY").expect("POSTIO_STORE_KEY is the hex store key");
    let key = StoreKey::from_hex(hex.trim()).expect("a 32-byte hex store key");

    let file = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let store = Store::open(&path, &key.derive(postio_storage::key::Purpose::Database))
        .await
        .expect("open the store");
    let connection = store.connect().await.expect("a connection");

    let count = async |sql: &str| -> i64 {
        postio_storage::sql::scalar(&connection, sql, ())
            .await
            .unwrap_or(-1)
    };

    let messages = count("SELECT count(*) FROM messages").await;
    let bodied = count("SELECT count(*) FROM messages WHERE body_text IS NOT NULL").await;
    let indexed = count("SELECT count(*) FROM messages WHERE body_search IS NOT NULL").await;
    let headers = count("SELECT count(*) FROM message_headers").await;
    let threads = count("SELECT count(*) FROM threads").await;
    let mailboxes = count("SELECT count(*) FROM mailboxes").await;
    let queued = count("SELECT count(*) FROM operation_queue").await;

    let page_size = count("PRAGMA page_size").await;
    let pages = count("PRAGMA page_count").await;
    let free = count("PRAGMA freelist_count").await;

    println!("file            {:>12}", bytes(file));
    println!(
        "pages           {pages:>12} x {page_size} B  ({} free, {})",
        free,
        bytes((free.max(0) as u64) * page_size.max(0) as u64)
    );
    println!("mailboxes       {mailboxes:>12}");
    println!("messages        {messages:>12}");
    println!("  with a body   {bodied:>12}");
    println!("  fts-indexed   {indexed:>12}");
    println!("threads         {threads:>12}");
    println!("header rows     {headers:>12}");
    println!("queued ops      {queued:>12}");
    // Where the bytes are. `dbstat` is gone on this engine (ADR 0038), so
    // this is the *content* each table holds rather than the pages it
    // occupies -- it cannot see index or page overhead, and the difference
    // between these sums and the file is exactly that overhead.
    let sum = async |sql: &str| -> i64 {
        postio_storage::sql::scalar(&connection, sql, ())
            .await
            .unwrap_or(-1)
    };
    let header_bytes =
        sum("SELECT coalesce(sum(length(name) + length(value)), 0) FROM message_headers").await;
    let body_bytes = sum("SELECT coalesce(sum(length(coalesce(body_text, '')) \
         + length(coalesce(body_html, ''))), 0) FROM messages")
    .await;
    let search_bytes =
        sum("SELECT coalesce(sum(length(coalesce(body_search, ''))), 0) FROM messages").await;
    let envelope_bytes = sum("SELECT coalesce(sum(length(coalesce(subject, '')) \
         + length(coalesce(preview, '')) + length(coalesce(remote_id, ''))), 0) FROM messages")
    .await;

    println!("\ncontent bytes (no index or page overhead):");
    println!("  header rows   {:>12}", bytes(header_bytes.max(0) as u64));
    println!("  bodies        {:>12}", bytes(body_bytes.max(0) as u64));
    println!("  fts source    {:>12}", bytes(search_bytes.max(0) as u64));
    println!(
        "  envelopes     {:>12}",
        bytes(envelope_bytes.max(0) as u64)
    );
    let content = (header_bytes + body_bytes + search_bytes + envelope_bytes).max(0) as u64;
    println!("  ---           {:>12}", bytes(content));
    if file > content {
        println!(
            "  overhead      {:>12}  ({:.0}% of the file)",
            bytes(file - content),
            (file - content) as f64 / file as f64 * 100.0
        );
    }

    // The interaction budget, on a real store rather than a fixture.
    let deep = std::time::Instant::now();
    let _ = count(
        "SELECT count(*) FROM (SELECT id FROM messages ORDER BY received_at DESC, id DESC \
         LIMIT 50 OFFSET 20000)",
    )
    .await;
    println!(
        "\na page 20,000 rows deep   {:>8.1} ms",
        deep.elapsed().as_secs_f64() * 1000.0
    );

    // The sixteen predicates ADR 0038 says were dropped, counted rather than
    // asserted: a partial index that lost its `WHERE` indexes every row
    // instead of the few that matched, and pays for it in pages.
    let indexes = count("SELECT count(*) FROM sqlite_master WHERE type = 'index'").await;
    let partial =
        count("SELECT count(*) FROM sqlite_master WHERE type = 'index' AND sql LIKE '%WHERE%'")
            .await;
    println!("\nindexes         {indexes:>12}  ({partial} still partial)");

    // Per folder: what is local, and whether a full sync ever finished for
    // it. A folder with rows but no `last_full_sync_at` is one the header
    // sync has not walked to the end of -- which is the difference between
    // "this mailbox is small" and "this mailbox is unfinished".
    println!("\nper folder:");
    println!(
        "{:>5}  {:>9}  {:>9}  {:>8}  {:>14}  path",
        "id", "local", "uid_next", "full?", "last attempt"
    );
    let rows = postio_storage::sql::all(
        &connection,
        "SELECT m.id, m.path, count(x.id), coalesce(s.uid_next, 0), coalesce(s.last_seen_at, 0), \
                CASE WHEN s.last_full_sync_at IS NULL THEN 0 ELSE 1 END \
           FROM mailboxes m \
           LEFT JOIN sync_state s ON s.mailbox_id = m.id \
           LEFT JOIN messages x ON x.mailbox_id = m.id \
          GROUP BY m.id ORDER BY count(x.id) DESC",
        (),
        |row| {
            Ok((
                row.col::<i64>(0)?,
                row.col::<String>(1)?,
                row.col::<i64>(2)?,
                row.col::<i64>(3)?,
                row.col::<i64>(4)?,
                row.col::<i64>(5)?,
            ))
        },
    )
    .await
    .unwrap_or_default();
    for (id, path, local, uid_next, seen, full) in rows {
        // `uid_next` is shown but deliberately not subtracted from: UIDs are
        // sparse, not dense. A folder that has had mail deleted out of it for
        // a decade has a high `uid_next` and few messages, and treating the
        // difference as "missing" reads a healthy mailbox as catastrophically
        // incomplete. What actually says a folder is unfinished is the
        // `full?` column -- no `last_full_sync_at` means the header sync has
        // never walked it to the end. Compare `local` against the server's
        // own `EXISTS`, which the sync logs at INFO.
        // `last_seen_at` is the attempt, `last_full_sync_at` the completion.
        // A folder that is being tried and failing has a fresh `seen` and no
        // `full`; one nothing is scheduling has neither moving.
        println!(
            "{id:>5}  {local:>9}  {uid_next:>9}  {:>8}  {:>14}  {path}",
            if full == 1 { "yes" } else { "NO" },
            if seen == 0 {
                "never".to_owned()
            } else {
                format!("{seen}")
            }
        );
    }

    if messages > 0 {
        println!(
            "\nper message     {:>12}",
            bytes(file / messages.max(1) as u64)
        );
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

/// `~` only. Enough for a path typed by hand at a shell.
fn shellexpand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => path.to_owned(),
    }
}
