//! What Postio's search looks like on Turso, with compression given up.
//!
//! ```sh
//! cd spike/turso && cargo run --release --bin uncompressed
//! ```
//!
//! Turso's full-text search indexes *columns*, and `messages.body_text` is a
//! zstd blob (ADR 0017). A column-indexing FTS cannot tokenise compressed
//! bytes, so using it means the body column holds plaintext. This builds both
//! stores over the same messages and shows what that trade looks like: the
//! size, and what a search actually returns.
//!
//! # What the size number here can and cannot claim
//!
//! `postio_storage::body`'s own doc says it plainly: **"generated mail
//! compresses 6-7x and that number means nothing"**. So the text here is the
//! real `.eml` corpus rather than anything synthesised — three dozen real
//! messages, repeated to fill a store. Repetition still flatters a
//! dictionary, so the ratio below is an *upper* bound on what compression is
//! worth and the reference figure stays the one `body.rs` records from a real
//! account: **2.19x on a 1.43 GB text axis**.
//!
//! What the comparison is honestly for is the *shape* — two stores, the same
//! messages, one with an FTS5 contentless index over compressed bodies and
//! one with a Turso fts index over plaintext ones — and what searching each
//! is like.

use std::time::Instant;

use turso::{Builder, EncryptionOpts};

const HEXKEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const CIPHER: &str = "aes256gcm";
/// How many messages to put in each store. The corpus is three dozen, so
/// this cycles it.
const MESSAGES: usize = 4_000;

/// The body text of every fixture in the corpus that has any.
fn corpus() -> Vec<(String, String)> {
    postio_model::test_corpus::all()
        .iter()
        .filter_map(|fixture| {
            let message = fixture.parse();
            let subject = message.subject.clone().unwrap_or_default();
            let body = message.body.text.clone()?;
            (!body.trim().is_empty()).then_some((subject, body))
        })
        .collect()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let texts = corpus();
    let bytes: usize = texts.iter().map(|(_, b)| b.len()).sum();
    println!(
        "corpus: {} fixtures with text, {:.1} KB, cycled to {MESSAGES} messages\n",
        texts.len(),
        bytes as f64 / 1024.0
    );

    let dir = std::env::temp_dir().join("postio-turso-uncompressed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;

    // ── A: what Postio has — SQLCipher, compressed bodies, FTS5 ──────────
    let plaintext_bytes;
    let sqlite_path = dir.join("sqlite/postio.db");
    std::fs::create_dir_all(sqlite_path.parent().unwrap())?;
    let key = postio_storage::key::StoreKey::generate();
    let (database, _blobs) = postio_session::open_store_at(&sqlite_path, &key)?;
    {
        let connection = database.connection()?;
        let account = postio_storage::test_support::account(&connection);
        let mailbox = postio_storage::test_support::mailbox(&connection, &account, "INBOX");
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let now = chrono::Utc::now();
        let mut plaintext = 0usize;
        for n in 0..MESSAGES {
            let (subject, body) = &texts[n % texts.len()];
            let mut message = postio_model::Message::new(account.id, mailbox.id, now);
            message.subject = Some(subject.clone());
            message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
                "<spike-{n}@example.com>"
            )));
            messages.create(&mut message)?;
            let stored = postio_storage::repository::StoredBody {
                text: Some(body.clone()),
                html: None,
                headers: None,
                headers_truncated: false,
                ..Default::default()
            };
            messages.set_body(message.id, &stored, postio_model::BodyState::Full)?;
            postio_index::index::index_body(&connection, message.id.get(), Some(body))?;
            plaintext += body.len();
        }
        plaintext_bytes = plaintext;
    }
    drop(database);
    let sqlite_bytes = store_size(&sqlite_path);

    // ── B: Turso, plaintext bodies, its own fts index ────────────────────
    let turso_path = dir.join("turso/postio.db");
    std::fs::create_dir_all(turso_path.parent().unwrap())?;
    let db = Builder::new_local(turso_path.to_str().unwrap())
        .experimental_encryption(true)
        .experimental_without_rowid(true)
        .experimental_index_method(true)
        .with_encryption(EncryptionOpts {
            cipher: CIPHER.to_string(),
            hexkey: HEXKEY.to_string(),
        })
        .build()
        .await?;
    let conn = db.connect()?;
    conn.execute(
        "CREATE TABLE messages(id INTEGER PRIMARY KEY, subject TEXT, body_text TEXT)",
        (),
    )
    .await?;
    conn.execute(
        "CREATE INDEX messages_fts ON messages USING fts (subject, body_text)",
        (),
    )
    .await?;
    for n in 0..MESSAGES {
        let (subject, body) = &texts[n % texts.len()];
        conn.execute(
            "INSERT INTO messages(id, subject, body_text) VALUES (?1, ?2, ?3)",
            (n as i64 + 1, subject.as_str(), body.as_str()),
        )
        .await?;
    }
    drop(conn);
    drop(db);
    let turso_bytes = store_size(&turso_path);

    // Comparing whole files would compare a full Postio store -- 40 tables,
    // 56 indexes, 15 triggers -- against a single three-column table, which
    // says nothing about compression. The body column is the thing that
    // changes, so the body column is what gets measured.
    let (stored, raw) = {
        let (database, _blobs) = postio_session::open_store_at(&sqlite_path, &key)?;
        let connection = database.connection()?;
        let stored: i64 = connection.query_row(
            "SELECT coalesce(sum(length(body_text)), 0) FROM messages",
            [],
            |row| row.get(0),
        )?;
        (stored as u64, plaintext_bytes as u64)
    };

    println!("── the body column, same {MESSAGES} messages ──");
    println!("  plaintext, as Turso would need it   {:>10} bytes", raw);
    println!(
        "  zstd + dictionary, as stored today  {:>10} bytes   {:.2}x smaller",
        stored,
        raw as f64 / stored.max(1) as f64
    );
    println!(
        "\n  Repeated corpus text flatters a dictionary, so read that ratio as an\n  \
         upper bound. `body.rs` records 2.19x from a real account's 1.43 GB of\n  \
         text, and warns that generated mail compresses 6-7x and means nothing."
    );
    println!(
        "\n  whole files, for completeness -- not a comparison, the schemas differ:\n  \
         sqlcipher {:.1} MB (full Postio schema)   turso {:.1} MB (one table)",
        sqlite_bytes as f64 / 1048576.0,
        turso_bytes as f64 / 1048576.0
    );

    // ── and what a search looks like on each ─────────────────────────────
    println!("\n── the same searches, over the same field, both engines ──");
    let db = Builder::new_local(turso_path.to_str().unwrap())
        .experimental_encryption(true)
        .with_encryption(EncryptionOpts {
            cipher: CIPHER.to_string(),
            hexkey: HEXKEY.to_string(),
        })
        .build()
        .await?;
    let conn = db.connect()?;
    let (database, _blobs) = postio_session::open_store_at(&sqlite_path, &key)?;
    let connection = database.connection()?;

    for term in ["invoice", "meeting", "thé", "the*"] {
        // FTS5
        let started = Instant::now();
        let fts5: Result<i64, _> = connection.query_row(
            "SELECT count(*) FROM message_bodies_fts WHERE message_bodies_fts MATCH ?1",
            [term],
            |row| row.get(0),
        );
        let fts5_took = started.elapsed();

        // Turso
        let started = Instant::now();
        let mut rows = conn
            .query(
                "SELECT count(*) FROM messages WHERE fts_match(body_text, ?1)",
                (term,),
            )
            .await;
        let turso_took = started.elapsed();
        let turso = match &mut rows {
            Ok(rows) => match rows.next().await {
                Ok(Some(row)) => row.get_value(0).map(|v| format!("{v:?}")).unwrap_or_default(),
                Ok(None) => "no rows".to_string(),
                Err(error) => format!("error: {error}"),
            },
            Err(error) => format!("error: {error}"),
        };

        println!(
            "  {term:<10} fts5 {:>18}  {fts5_took:>9.2?}     turso {:<28} {turso_took:>9.2?}",
            match &fts5 {
                Ok(n) => format!("{n} hits"),
                Err(e) => format!("error: {e}"),
            },
            turso,
        );
    }

    Ok(())
}

/// A store is its file plus whatever the journal left beside it.
fn store_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Some(dir) = path.parent() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}
