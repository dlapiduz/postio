//! Postio's real schema and real queries, on Turso, with encryption on.
//!
//! ```sh
//! cd spike/turso && cargo run --release
//! ```
//!
//! A compatibility report, not a rewrite. What a rewrite needs answered first
//! is how much of what `postio-storage` already asks of SQLite, Turso can do
//! — so this does not invent a schema. It builds a real Postio store through
//! `postio_session::open_store_at`, reads the head schema back out of
//! `sqlite_schema`, and replays that against Turso: every table, index,
//! trigger and FTS5 virtual table the application actually creates.
//!
//! The head schema rather than the twenty migrations, deliberately. A store
//! rebuilt from scratch is created at head; replaying `ALTER TABLE` history
//! would measure Turso against migrations nobody would run again, and would
//! report failures that a rewrite would never meet.
//!
//! Both features this leans on are experimental upstream — encryption at
//! rest, and full-text search. That is the reason to run it rather than read
//! about it.

use turso::{Builder, EncryptionOpts};

const HEXKEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const CIPHER: &str = "aes256gcm";

/// One object out of `sqlite_schema`.
struct Object {
    kind: String,
    name: String,
    sql: String,
}

/// Build a real Postio store and read its head schema back.
fn head_schema() -> (Vec<Object>, usize) {
    let directory = tempfile::tempdir().expect("a directory");
    let key = postio_storage::key::StoreKey::generate();
    let (database, _blobs) =
        postio_session::open_store_at(directory.path().join("postio.db"), &key)
            .expect("the application's own store");
    let connection = database.connection().expect("a connection");
    let mut statement = connection
        .prepare(
            "SELECT type, name, sql FROM sqlite_schema
              WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
              ORDER BY rowid",
        )
        .expect("the schema query");
    let mut objects: Vec<Object> = statement
        .query_map([], |row| {
            Ok(Object {
                kind: row.get(0)?,
                name: row.get(1)?,
                sql: row.get(2)?,
            })
        })
        .expect("read the schema")
        .filter_map(Result::ok)
        .collect();
    // FTS5's shadow tables are SQLite's to create, not the application's: a
    // rebuilt store issues `CREATE VIRTUAL TABLE` and gets `_data`, `_idx`,
    // `_content`, `_docsize` and `_config` for free. Counting them as things
    // Turso refused would be counting the same absence five times.
    let virtuals: Vec<String> = objects
        .iter()
        .filter(|o| o.sql.to_uppercase().contains("USING FTS5"))
        .map(|o| o.name.clone())
        .collect();
    objects.retain(|o| {
        !virtuals.iter().any(|v| {
            ["_data", "_idx", "_content", "_docsize", "_config"]
                .iter()
                .any(|suffix| o.name == format!("{v}{suffix}"))
        })
    });
    let tables: usize = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;
    (objects, tables)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("── Postio's head schema, out of a real store ──");
    let (objects, tables) = head_schema();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for object in &objects {
        *counts.entry(object.kind.clone()).or_default() += 1;
    }
    println!(
        "   {} objects: {}",
        objects.len(),
        counts
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("   ({tables} tables, FTS5 virtual tables included)\n");

    let dir = std::env::temp_dir().join("postio-turso-spike");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("postio.db");

    println!("── Turso, encryption on ──");
    let db = Builder::new_local(path.to_str().expect("utf-8"))
        .experimental_encryption(true)
        .experimental_without_rowid(true)
        .experimental_triggers(true)
        .experimental_generated_columns(true)
        .experimental_index_method(true)
        .experimental_strict(true)
        .with_encryption(EncryptionOpts {
            cipher: CIPHER.to_string(),
            hexkey: HEXKEY.to_string(),
        })
        .build()
        .await?;
    let conn = db.connect()?;
    println!("   opened with cipher {CIPHER}\n");

    // ── the schema ───────────────────────────────────────────────────────
    let mut failed: Vec<(&Object, String)> = Vec::new();
    let mut applied = 0usize;
    for object in &objects {
        match conn.execute(&object.sql, ()).await {
            Ok(_) => applied += 1,
            Err(error) => failed.push((object, error.to_string())),
        }
    }

    println!("── applying it ──");
    println!("   {applied} applied, {} refused", failed.len());
    if !failed.is_empty() {
        let mut by_kind = std::collections::BTreeMap::<String, Vec<&Object>>::new();
        for (object, _) in &failed {
            by_kind.entry(object.kind.clone()).or_default().push(object);
        }
        println!("\n   refused, by kind:");
        for (kind, objects) in &by_kind {
            let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).take(6).collect();
            println!(
                "     {:<8} {:>3}   {}{}",
                kind,
                objects.len(),
                names.join(", "),
                if objects.len() > names.len() {
                    ", …"
                } else {
                    ""
                }
            );
        }
        println!("\n   distinct errors:");
        let mut seen = std::collections::BTreeMap::<String, usize>::new();
        for (_, error) in &failed {
            *seen.entry(error.clone()).or_default() += 1;
        }
        for (error, count) in seen.iter().take(12) {
            println!("     {count:>3}x  {error}");
        }
    }

    // ── what Turso offers instead of FTS5 ────────────────────────────────
    //
    // Not a virtual table and not `MATCH`: an *index method* over an ordinary
    // table, queried through functions. Shown working, because "FTS5 is
    // missing" and "there is no full-text search" are different findings and
    // only one of them is true.
    println!("\n── Turso's own full-text search ──");
    conn.execute(
        "CREATE TABLE mail(id INTEGER PRIMARY KEY, subject TEXT, body TEXT)",
        (),
    )
    .await?;
    match conn
        .execute(
            "CREATE INDEX mail_fts ON mail USING fts (subject, body)",
            (),
        )
        .await
    {
        Ok(_) => println!("   CREATE INDEX … USING fts (subject, body)   ok"),
        Err(error) => println!("   CREATE INDEX … USING fts refused: {error}"),
    }
    conn.execute(
        "INSERT INTO mail(subject, body) VALUES ('the invoice you asked for', 'attached is the invoice for March')",
        (),
    )
    .await?;
    conn.execute(
        "INSERT INTO mail(subject, body) VALUES ('lunch on Thursday', 'are you free at one')",
        (),
    )
    .await?;

    let mut rows = conn
        .query(
            "SELECT subject, fts_score(subject, body, 'invoice') FROM mail
              WHERE fts_match(subject, body, 'invoice')",
            (),
        )
        .await?;
    let mut hits = 0;
    while let Some(row) = rows.next().await? {
        hits += 1;
        println!(
            "   hit: {:?}  score {:?}",
            row.get_value(0)?,
            row.get_value(1)?
        );
    }
    println!("   {hits} hit(s) — through fts_match/fts_score, not MATCH/bm25");

    // ── and the encryption, which is the other experimental half ─────────
    println!("\n── the encryption ──");
    drop(rows);
    drop(conn);
    drop(db);
    let bytes = std::fs::read(&path)?;
    let plaintext = bytes.starts_with(b"SQLite format 3");
    println!("   header       = {:02x?}", &bytes[..16.min(bytes.len())]);
    println!("   plaintext    = {plaintext}");
    let leaked: Vec<&str> = ["invoice", "Thursday", "CREATE TABLE"]
        .into_iter()
        .filter(|m| {
            bytes
                .windows(m.len())
                .any(|w| w.eq_ignore_ascii_case(m.as_bytes()))
        })
        .collect();
    println!(
        "   scan         = {}",
        if leaked.is_empty() {
            "no plaintext found".to_string()
        } else {
            format!("FOUND IN THE CLEAR: {leaked:?}")
        }
    );

    let wrong = Builder::new_local(path.to_str().expect("utf-8"))
        .experimental_encryption(true)
        .with_encryption(EncryptionOpts {
            cipher: CIPHER.to_string(),
            hexkey: "ff".repeat(32),
        })
        .build()
        .await;
    let refused = match wrong {
        Err(_) => true,
        Ok(db) => match db.connect() {
            Err(_) => true,
            Ok(c) => c.query("SELECT count(*) FROM mail", ()).await.is_err(),
        },
    };
    println!(
        "   wrong key    = {}",
        if refused {
            "refused"
        } else {
            "OPENED THE STORE"
        }
    );

    Ok(())
}
