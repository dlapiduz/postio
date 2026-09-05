//! The two numbers #381 asks for: what the partial index saves, and what
//! `cipher_page_size = 8192` would buy (ADR 0017 axis 3).
//!
//! Ignored by default: it seeds a realistic store and vacuums it three times,
//! which is minutes, and its output is numbers for a person to read rather
//! than an assertion. Run it with
//!
//! ```text
//! cargo test -p postio-storage --test storage_suite page_size -- --ignored --nocapture
//! ```
//!
//! # Why it measures a `VACUUM` rather than two fresh stores
//!
//! Because that is what adopting 8192 would have to do. SQLCipher's page size
//! is fixed when the file is created and is **not** discoverable from the
//! file: a store written at 8192 and opened by a connection that does not say
//! `cipher_page_size = 8192` answers *"file is not a database"*, which
//! [`postio_storage::Error::WrongStoreKey`] turns into "your store will not
//! decrypt". So the only way to move an existing store is to set the pragma
//! and `VACUUM`, and the only honest control is the same store vacuumed at
//! 4096 — otherwise the comparison measures compaction and calls it page
//! size.

use std::path::Path;

use postio_model::BodyState;
use postio_model::ids::MessageId;
use postio_storage::repository::StoredBody;
use postio_storage::seed::seed_large;
use postio_storage::{Database, test_support};

/// Messages to seed. The reference store this issue was measured on holds
/// 81,744; this is a quarter of it, which is enough for the b-tree shape to
/// be the thing being measured rather than the overhead.
const MESSAGES: usize = 20_000;

/// A body with the shape real mail has: prose, repetitive enough to
/// compress the way a real message does, long enough to leave the page it
/// starts on.
fn body_text(n: usize) -> String {
    let mut text = String::with_capacity(3_000);
    for line in 0..40 {
        text.push_str(&format!(
            "This is line {line} of message {n}. It says something ordinary, \
             at about the length an ordinary sentence in an ordinary email \
             runs to before it wraps.\n"
        ));
    }
    text
}

fn bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

/// A pragma that answers a number, however SQLCipher chooses to type it.
///
/// `PRAGMA page_size` on a keyed connection comes back as a *text* column
/// named `cipher_page_size` — SQLCipher answers it with its own handler — so
/// asking rusqlite for an `i64` fails with `InvalidColumnType` on a value
/// that is plainly a number.
fn number(connection: &rusqlite::Connection, pragma: &str) -> i64 {
    let raw: rusqlite::types::Value = connection
        .query_row(pragma, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("{pragma}: {error}"));
    match raw {
        rusqlite::types::Value::Integer(value) => value,
        rusqlite::types::Value::Text(text) => text.trim().parse().expect("a number"),
        other => panic!("{pragma} answered {other:?}"),
    }
}

/// `page_count * page_size`, and the free pages inside it.
fn pages(connection: &rusqlite::Connection) -> (i64, i64, i64) {
    (
        number(connection, "PRAGMA page_size"),
        number(connection, "PRAGMA page_count"),
        number(connection, "PRAGMA freelist_count"),
    )
}

#[test]
#[ignore = "#381: a measurement, not an assertion -- minutes, and its output is numbers"]
fn page_size_8192_against_4096_on_a_realistic_store() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("store.db");
    let key = test_support::key();

    let seeded = {
        let database = Database::open(&path, &key).expect("a store");
        let report = seed_large(&database, 7, MESSAGES);
        database
            .connection()
            .expect("a connection")
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint, so this measures the database and not its log");
        report
    };
    // Bodies, because without them this measures a store a third of the
    // weight of a real one: the reference store is ~2 KB per message and a
    // body-less seed is ~580 bytes, and the difference is exactly the
    // compressed body columns (ADR 0020) — the rows most likely to spill to
    // overflow pages, which is where a bigger page could plausibly help.
    {
        let database = Database::open(&path, &key).expect("a store");
        let connection = database.connection().expect("a connection");
        let repository = postio_storage::repository::MessageRepository::new(&connection);
        let ids: Vec<i64> = connection
            .prepare("SELECT id FROM messages ORDER BY id")
            .expect("a statement")
            .query_map([], |row| row.get(0))
            .expect("ids")
            .collect::<Result<_, _>>()
            .expect("ids");
        for (n, id) in ids.iter().enumerate() {
            let body = StoredBody {
                text: Some(body_text(n)),
                html: Some(format!("<html><body><p>{}</p></body></html>", body_text(n))),
                headers: Some(format!(
                    "From: sender{n}@example.com\r\nTo: reader@example.test\r\n\
                     Subject: message {n}\r\nDate: Mon, 1 Jun 2026 09:0{}:00 +0000\r\n",
                    n % 10
                )),
                headers_truncated: false,
                encoding_problems: false,
            };
            repository
                .set_body(MessageId::new(*id), &body, BodyState::Full)
                .expect("a body");
        }
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint");
        println!("wrote {} bodies", ids.len());
    }

    println!("seeded {} messages", seeded.message_count);
    println!("as written (4096):   {:>12} bytes", bytes(&path));

    // The control: the same store, compacted, at the page size it already
    // has. Everything below is measured against *this*, not against the line
    // above, or compaction gets credited to page size.
    let connection = rusqlite::Connection::open(&path).expect("a raw connection");
    postio_storage::db::configure(&connection, &key).expect("keyed");
    connection.execute_batch("VACUUM;").expect("vacuum at 4096");
    let (size, count, free) = pages(&connection);
    println!(
        "vacuumed  (4096):   {:>12} bytes  ({count} pages of {size}, {free} free)",
        bytes(&path)
    );

    // The conversion. `PRAGMA cipher_page_size = 8192; VACUUM;` is the
    // obvious spelling and it silently does nothing -- measured: the file
    // came back at the same 2,835 pages of 4,096. SQLCipher's own path for
    // changing cipher settings on a store that already exists is
    // `sqlcipher_export` into a database attached with the new settings,
    // which is a full rewrite of every page, which is the point: this is
    // #300's pass, not a maintenance job.
    let converted = directory.path().join("converted.db");
    let hex = key.to_hex();
    connection
        .execute_batch(&format!(
            "ATTACH DATABASE {} AS converted KEY \"x'{}'\";",
            quoted(&converted),
            *hex
        ))
        .expect("attach the destination");
    connection
        .execute_batch("PRAGMA converted.cipher_page_size = 8192;")
        .expect("the destination's page size, before anything is written to it");
    connection
        .query_row("SELECT sqlcipher_export('converted')", [], |_| Ok(()))
        .expect("export");
    connection
        .execute_batch("DETACH DATABASE converted;")
        .expect("detach");
    drop(hex);
    drop(connection);

    let connection = rusqlite::Connection::open(&converted).expect("a raw connection");
    keyed_at(&connection, &key, 8192);
    let (size, count, free) = pages(&connection);
    println!(
        "exported  (8192):   {:>12} bytes  ({count} pages of {size}, {free} free)",
        bytes(&converted)
    );
    drop(connection);

    // And what it costs to be wrong about this. An existing store and a build
    // that disagree about the page size do not degrade: they refuse, and the
    // refusal reads as "your mail will not decrypt".
    let connection = rusqlite::Connection::open(&converted).expect("a raw connection");
    println!(
        "the 8192 store, opened by a build that does not say 8192: {}",
        match postio_storage::db::configure(&connection, &key) {
            Ok(()) => "opened".to_string(),
            Err(error) => format!("{error}"),
        }
    );
}

/// A path as a SQL string literal.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// Key a raw connection the way [`postio_storage::db::configure`] does, but
/// declaring a page size — the thing `configure` has no way to say.
fn keyed_at(connection: &rusqlite::Connection, key: &postio_storage::key::Subkey, page: u32) {
    connection
        .execute_batch("PRAGMA cipher_memory_security = OFF;")
        .expect("memory security off");
    let hex = key.to_hex();
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", *hex))
        .expect("key");
    drop(hex);
    connection
        .execute_batch(&format!("PRAGMA cipher_page_size = {page};"))
        .expect("page size");
}

#[test]
#[ignore = "#381: a measurement, not an assertion -- its output is numbers"]
fn what_the_partial_draft_index_saves() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("store.db");
    let key = test_support::key();
    let database = Database::open(&path, &key).expect("a store");
    let seeded = seed_large(&database, 7, MESSAGES);

    let connection = database.connection().expect("a connection");
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM attachments", [], |row| row.get(0))
        .expect("a count");
    let drafts: i64 = connection
        .query_row(
            "SELECT count(*) FROM attachments WHERE draft_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("a count");
    println!(
        "{} messages, {rows} attachment rows, {drafts} of them a draft's",
        seeded.message_count
    );

    // Measured as pages, because that is what an index costs. The store is
    // vacuumed either side so the number is the index and not whatever the
    // freelist happened to be holding.
    let cost = |sql: &str| -> i64 {
        connection
            .execute_batch(&format!(
                "DROP INDEX IF EXISTS idx_attachments_draft; {sql} VACUUM;"
            ))
            .expect("rebuild the index");
        number(&connection, "PRAGMA page_count")
    };

    let partial = cost(
        "CREATE INDEX idx_attachments_draft ON attachments (draft_id, position) \
         WHERE draft_id IS NOT NULL;",
    );
    let whole = cost("CREATE INDEX idx_attachments_draft ON attachments (draft_id, position);");
    let page = number(&connection, "PRAGMA page_size");

    println!("partial: {partial} pages of {page}");
    println!("whole:   {whole} pages of {page}");
    println!(
        "saved:   {} pages, {} bytes, over {rows} rows -- {:.1} bytes a row",
        whole - partial,
        (whole - partial) * page,
        ((whole - partial) * page) as f64 / rows as f64
    );
}
