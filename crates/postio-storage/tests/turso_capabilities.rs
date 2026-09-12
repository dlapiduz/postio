//! What the engine can actually do, established by asking it.
//!
//! Two of these are the Phase 0 experiments from `specs/004-turso-store`
//! (**R1** and **R2**), and they are tests rather than a scratch program for
//! one reason: their answers are written into `research.md` and three later
//! tasks branch on them, so the answer has to stay true. A capability that was
//! probed once in a terminal is a memory; a capability with a test is a fact
//! that fails loudly when the pre-1.0 engine under it changes its mind.
//!
//! The rest prove the properties `spec.md` makes acceptance criteria: that the
//! store opens, that it is encrypted, and that another key is refused.

use postio_storage::Store;
use postio_storage::key::{Purpose, StoreKey};
use postio_storage::store::CIPHER;

fn temp(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(name);
    (dir, path)
}

fn a_key() -> postio_storage::key::Subkey {
    StoreKey::generate().derive(Purpose::Database)
}

/// Read a single-row, single-column count, and let go of the connection.
///
/// The `drop` is not tidiness. A `Rows` holds its connection's operation gate
/// until it is dropped or drained, and the next statement on that connection
/// fails with `Misuse("connection is busy with another operation")` -- at
/// runtime, with no borrow to stop it at compile time the way `rusqlite`'s
/// `Statement` did. It cost an afternoon of reading a contaminated experiment
/// to find, so it is written down here and in research.md Q2.
async fn count(connection: &postio_storage::Connection, sql: &str) -> i64 {
    let mut rows = connection.query(sql, ()).await.expect("query");
    let row = rows.next().await.expect("row").expect("a count row");
    let value = *row.get_value(0).expect("column").as_integer().expect("integer");
    drop(rows);
    value
}

/// T007: a fresh path yields a store whose schema is at head.
#[tokio::test]
async fn opening_a_fresh_path_puts_the_schema_at_head() {
    let (_dir, path) = temp("fresh.db");
    let store = Store::open(&path, &a_key()).await.expect("open");
    let connection = store.connect().await.expect("connect");

    let mut rows = connection
        .query(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            (),
        )
        .await
        .expect("read the schema");

    let mut found = Vec::new();
    while let Some(row) = rows.next().await.expect("row") {
        found.push(row.get_value(0).expect("name").as_text().expect("text").clone());
    }
    found.sort();

    // The tables the application cannot start without, not the whole list:
    // `schema.rs`'s own test is what proves nothing was lost. This one proves
    // the constant reached the engine.
    for expected in ["accounts", "mailboxes", "messages", "threads", "operation_queue"] {
        assert!(
            found.iter().any(|name| name == expected),
            "the store opened without `{expected}`; it has {found:?}",
        );
    }
    assert!(
        found.len() >= 24,
        "expected the whole schema, found only {} tables: {found:?}",
        found.len(),
    );
}

/// SC-002: the file on disk is not readable as plaintext.
#[tokio::test]
async fn the_store_keeps_no_plaintext_on_disk() {
    let (_dir, path) = temp("secret.db");
    let store = Store::open(&path, &a_key()).await.expect("open");
    let connection = store.connect().await.expect("connect");

    // A string that cannot plausibly occur in schema text or engine padding.
    const NEEDLE: &str = "asparagus-turntable-9417-zzyzx";
    connection
        .execute(
            "INSERT INTO settings (key, account_id, value, updated_at)
             VALUES ('probe', NULL, ?1, 0)",
            turso::params![NEEDLE],
        )
        .await
        .expect("write the probe");
    connection
        .execute("PRAGMA wal_checkpoint(TRUNCATE)", ())
        .await
        .ok();
    drop(connection);
    drop(store);

    for candidate in [path.clone(), path.with_extension("db-wal")] {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        assert!(
            !bytes
                .windows(NEEDLE.len())
                .any(|window| window == NEEDLE.as_bytes()),
            "{} contains the probe string in plaintext -- the store is not encrypted",
            candidate.display(),
        );
    }
}

/// SC-003: a store will not open under a key that is not its own.
#[tokio::test]
async fn another_key_is_refused() {
    let (_dir, path) = temp("keyed.db");
    let store = Store::open(&path, &a_key()).await.expect("open");
    drop(store);

    let outcome = Store::open(&path, &a_key()).await;
    assert!(
        matches!(outcome, Err(postio_storage::Error::WrongStoreKey)),
        "a store opened under a different key should be WrongStoreKey, got {:?}",
        outcome.map(|_| "Ok"),
    );
}

/// **R1** — can an fts index be built over a generated column?
///
/// The question T033 and T034 branch on. The mechanical answer is **yes**:
/// the engine builds the index and matches through it. That is not the end of
/// it, and the second half is why `body_search` is an ordinary column anyway
/// -- see `fold_cannot_be_expressed_in_sql` below.
///
/// `STORED` is refused ("Stored generated columns are not supported"); only
/// `VIRTUAL` works.
#[tokio::test]
async fn r1_an_fts_index_can_be_built_over_a_generated_column() {
    let (_dir, path) = temp("r1.db");
    let store = Store::open(&path, &a_key()).await.expect("open");
    let connection = store.connect().await.expect("connect");

    connection
        .execute_batch(
            "CREATE TABLE probe (
                 id           INTEGER PRIMARY KEY,
                 body_text    TEXT,
                 body_search  TEXT,
                 body_indexed TEXT GENERATED ALWAYS AS (coalesce(body_search, body_text)) VIRTUAL
             );
             CREATE INDEX probe_fts ON probe USING fts (body_indexed);",
        )
        .await
        .expect("an fts index over a VIRTUAL generated column");

    connection
        .execute(
            "INSERT INTO probe (body_text, body_search) VALUES ('a wombat appears', NULL)",
            (),
        )
        .await
        .expect("insert");

    let matched = count(
        &connection,
        "SELECT count(*) FROM probe WHERE fts_match(body_indexed, 'wombat')",
    )
    .await;
    assert_eq!(
        matched, 1,
        "the index was built over the generated column but did not match \
         through it -- R1's answer has changed",
    );
}

/// Why R1 being "yes" does not make the fold free.
///
/// A generated column can only be an SQL expression, and the fold this needs
/// -- NFD, then drop the combining marks -- has no SQL spelling here. The
/// engine's string functions are SQLite's, and `unicode()` returns a
/// codepoint while `unistr()` decodes escapes; neither normalises. So
/// `body_search` is written by the application on the way in, and
/// `postio_index::fold` is the one place that knows how.
///
/// Recorded as a test because it is the load-bearing half of the decision: if
/// a normalising function ever appears, this fails and the column can become
/// generated.
#[tokio::test]
async fn fold_cannot_be_expressed_in_sql() {
    let (_dir, path) = temp("fold.db");
    let store = Store::open(&path, &a_key()).await.expect("open");
    let connection = store.connect().await.expect("connect");

    for spelling in ["nfd(?1)", "unaccent(?1)", "normalize(?1, 'NFD')", "icu_fold(?1)"] {
        let outcome = connection
            .query(&format!("SELECT {spelling}"), turso::params!["café"])
            .await;
        assert!(
            outcome.is_err(),
            "`{spelling}` resolved -- the engine has grown a normalising \
             function, so `body_search` could become a generated column after \
             all (R1, research.md Q1)",
        );
    }
}

/// Case *is* folded; diacritics are not.
///
/// The precise shape of what the application has to make up for. The analyzer
/// behind `USING fts` is tantivy's `SimpleTokenizer` + `LowerCaser`, so `CAFÉ`
/// finds `café` and `cafe` does not -- which is the whole of FR-012 and the
/// reason `fold` exists.
#[tokio::test]
async fn the_engine_folds_case_but_not_diacritics() {
    let (_dir, path) = temp("accents.db");
    let store = Store::open(&path, &a_key()).await.expect("open");
    let connection = store.connect().await.expect("connect");

    connection
        .execute_batch(
            "CREATE TABLE accents (id INTEGER PRIMARY KEY, body TEXT);
             CREATE INDEX accents_fts ON accents USING fts (body);",
        )
        .await
        .expect("create");
    connection
        .execute("INSERT INTO accents (body) VALUES ('un café très noir')", ())
        .await
        .expect("insert");

    let hits = |term: &'static str| {
        let connection = connection.clone();
        async move {
            count(
                &connection,
                &format!("SELECT count(*) FROM accents WHERE fts_match(body, '{term}')"),
            )
            .await
        }
    };

    assert_eq!(hits("café").await, 1, "the exact term should match");
    assert_eq!(hits("CAFÉ").await, 1, "case is folded by the analyzer");
    assert_eq!(
        hits("cafe").await,
        0,
        "if this is 1 the engine has started folding diacritics, and \
         `body_search` plus `postio_index::fold` are no longer needed",
    );
    assert_eq!(hits("tres").await, 0, "same, for a grave accent");
}

/// **R2** — can a background writer starve an interactive one?
///
/// The question T014 branches on: whether `WritePriority` has to be enforced
/// by an application-level gate, as it was under the old engine, or whether
/// the engine's own writer scheduling already bounds how long a short write
/// waits behind a long one.
///
/// Deliberately not a timing assertion — a shared runner cannot defend a
/// millisecond figure, which is the same reason `bench.yml` times nothing.
/// What it asserts is *completion*: the short write finishes while the long
/// one is still going, or it does not.
#[tokio::test]
async fn r2_whether_a_long_write_blocks_a_short_one() {
    let (_dir, path) = temp("r2.db");
    let store = Store::open(&path, &a_key()).await.expect("open");

    let long = store.connect().await.expect("connect");
    let short = store.connect().await.expect("connect");

    long.execute("BEGIN IMMEDIATE", ())
        .await
        .expect("take the writer");
    long.execute(
        "INSERT INTO settings (key, account_id, value, updated_at)
         VALUES ('long', NULL, 'x', 0)",
        (),
    )
    .await
    .expect("write inside the transaction");

    // The interactive write, with a bound on how long we are willing to call
    // "waiting" rather than "starved".
    let attempt = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        short.execute(
            "INSERT INTO settings (key, account_id, value, updated_at)
             VALUES ('short', NULL, 'y', 0)",
            (),
        ),
    )
    .await;

    long.execute("COMMIT", ()).await.expect("release");

    match attempt {
        Ok(Ok(_)) => panic!(
            "R2 has changed: a second connection completed a write while the \
             first held an IMMEDIATE transaction. research.md Q3 records that \
             it cannot, and the write gate exists because of it.",
        ),
        Ok(Err(error)) => {
            // Refused rather than queued: the engine reports the busy writer
            // instead of waiting for it. Either way the interactive write did
            // not go through, which is what the gate is for.
            let said = error.to_string().to_lowercase();
            assert!(
                said.contains("busy") || said.contains("lock") || said.contains("write"),
                "the second write failed for an unexpected reason: {error}",
            );
        }
        Err(_elapsed) => {
            // Queued behind the long write and still waiting. This is the
            // starvation R2 is about.
        }
    }
}

/// The cipher the store is opened under is the one this crate documents.
///
/// Cheap, and it catches a rename in a pre-1.0 dependency that would
/// otherwise fall back to *no* encryption without anything failing.
#[tokio::test]
async fn the_documented_cipher_is_one_the_engine_accepts() {
    let (_dir, path) = temp("cipher.db");
    // If CIPHER were not a name the engine knows, opening would fail here.
    Store::open(&path, &a_key()).await.expect("open");
    assert_eq!(CIPHER, "aes256gcm");
}
