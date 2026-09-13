//! Read-only: how much of the `messages` table is indexed body text, and what
//! that costs the queries that never read one. Sizes and counts only.
//!
//! **Point this at a copy.** There is no read-only open in the engine's Rust
//! API, so the safety is this instruction rather than the file mode. Nothing
//! below writes; what a copy protects against is the engine's own recovery on
//! open.
//!
//! ```sh
//! POSTIO_DIAG_KEY=$(secret-tool lookup application postio account "local store encryption key") \
//!   cargo run --release -p postio-runtime --example table_shape -- /path/to/copy/postio.db
//! ```
//!
//! # What this can no longer say
//!
//! The last section used to break the file down by b-tree, from `dbstat`.
//! This engine has no `dbstat` and no per-object page accounting at all, so
//! "where the database goes" is now answerable only as a whole-file delta
//! between two states — which is what `postio-index`'s size measurements do.
use postio_storage::key::{Purpose, StoreKey};

#[tokio::main]
async fn main() {
    let master = StoreKey::from_hex(std::env::var("POSTIO_DIAG_KEY").unwrap().trim()).unwrap();
    let key = master.derive(Purpose::Database);
    let path = std::env::args().nth(1).unwrap();
    let store = postio_storage::Store::open(&path, &key)
        .await
        .expect("open the store");
    let connection = store.connect().await.expect("a connection");

    let page = postio_storage::sql::scalar(&connection, "PRAGMA page_size", ())
        .await
        .unwrap_or(4096);
    println!("page size {page} bytes");

    // `body_search` is the whole of the body text in a `messages` row now:
    // ADR 0020's `body_text`/`body_html`/`body_headers` columns are gone, and
    // the one that is left is the folded text the full-text index is built
    // over (specs/004-turso-store).
    let (rows, body, total): (i64, i64, i64) = postio_storage::sql::one(
        &connection,
        "SELECT count(*),
                coalesce(sum(length(coalesce(body_search,''))), 0),
                coalesce(sum(length(coalesce(subject,'')) + length(coalesce(preview,''))
                           + length(coalesce(rfc_message_id,'')) + 120), 0)
           FROM messages",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((row.col(0)?, row.col(1)?, row.col(2)?))
        },
    )
    .await
    .expect("read the table shape");
    if rows == 0 {
        println!("messages: no rows");
        return;
    }
    println!("messages: {rows} rows");
    println!(
        "  body_search:       {:>10} bytes ({:.1} MiB), mean {:.0}/row",
        body,
        body as f64 / 1048576.0,
        body as f64 / rows as f64
    );
    println!(
        "  everything else:  ~{:>10} bytes ({:.1} MiB), mean {:.0}/row",
        total,
        total as f64 / 1048576.0,
        total as f64 / rows as f64
    );
    println!(
        "  body is {:.0}% of the table",
        100.0 * body as f64 / (body + total).max(1) as f64
    );

    // Rows per page, with and without the body text: the number that decides
    // how many pages a list or a hydrate has to read and decrypt.
    let with = page as f64 / ((body + total) as f64 / rows as f64);
    let without = page as f64 / (total as f64 / rows as f64);
    println!(
        "  rows per {page}-byte page: {:.1} as stored, {:.1} if the text moved out ({:.1}x)",
        with,
        without,
        without / with
    );

    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("\nthe file itself: {:.1} MiB", bytes as f64 / 1048576.0);
}
