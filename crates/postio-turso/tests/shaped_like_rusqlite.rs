//! The shim, against the shapes `postio-storage` actually uses.
//!
//! Not a test of Turso. A test that the *call sites* compile and behave:
//! every construction here is copied from the storage layer's own style —
//! `prepare` then `query_map` then `collect`, `query_row` with a closure,
//! `params!` with mixed types, `optional()` on a missing row, and `get` by
//! index and by name.
//!
//! If this file stops compiling, so do several hundred lines that were
//! supposed to move engine without moving.

use postio_turso::{Connection, Encryption, OptionalExtension, Result, params};

/// A message row as the storage layer reads one: every column type the
/// repositories actually bind and read, in one tuple.
type MessageRow = (i64, String, u32, bool, Option<Vec<u8>>);

const HEXKEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn store(directory: &std::path::Path, name: &str) -> Connection {
    Connection::open(
        directory.join(name).to_str().expect("utf-8"),
        Some(Encryption {
            cipher: "aes256gcm".to_owned(),
            hexkey: HEXKEY.to_owned(),
        }),
    )
    .expect("open")
}

#[test]
fn the_call_shapes_the_storage_layer_uses() -> Result<()> {
    let directory = tempfile::tempdir().expect("a directory");
    let connection = store(directory.path(), "postio.db");

    connection.execute_batch(
        "CREATE TABLE messages(
            id       INTEGER PRIMARY KEY,
            subject  TEXT NOT NULL,
            size     INTEGER NOT NULL,
            seen     INTEGER NOT NULL DEFAULT 0,
            body     BLOB
         );",
    )?;

    // `execute` with `params!`, mixed types and a NULL.
    let changed = connection.execute(
        "INSERT INTO messages(id, subject, size, seen, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![1i64, "a first message", 4096i64, true, None::<Vec<u8>>],
    )?;
    assert_eq!(changed, 1);

    connection.execute(
        "INSERT INTO messages(id, subject, size, seen, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![2i64, "a reply", 512i64, false, vec![0xdeu8, 0xad]],
    )?;

    // `query_row` with a closure, the shape half the repositories use.
    let subject: String = connection.query_row(
        "SELECT subject FROM messages WHERE id = ?1",
        [1i64],
        |row| row.get(0),
    )?;
    assert_eq!(subject, "a first message");

    // `prepare` then `query_map` then `collect`, the other half.
    let mut statement =
        connection.prepare("SELECT id, subject, size, seen, body FROM messages ORDER BY id")?;
    let rows: Vec<MessageRow> = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, "a first message");
    assert_eq!(rows[0].2, 4096);
    assert!(rows[0].3, "a bound `true` reads back as one");
    assert_eq!(rows[0].4, None, "a bound NULL reads back as None");
    assert_eq!(rows[1].4, Some(vec![0xde, 0xad]));

    // By name as well as by index: both spellings are in the storage layer.
    let by_name: String = connection.query_row(
        "SELECT subject FROM messages WHERE id = ?1",
        [2i64],
        |row| row.get("subject"),
    )?;
    assert_eq!(by_name, "a reply");

    // `optional()`, which is how a missing row is a `None` rather than an
    // error in six places.
    let missing: Option<String> = connection
        .query_row(
            "SELECT subject FROM messages WHERE id = ?1",
            [99i64],
            |row| row.get(0),
        )
        .optional()?;
    assert_eq!(missing, None);

    // `last_insert_rowid`, which every `create` reads.
    connection.execute(
        "INSERT INTO messages(subject, size) VALUES ('third', 1)",
        (),
    )?;
    assert_eq!(connection.last_insert_rowid(), 3);

    Ok(())
}

#[test]
fn the_store_is_encrypted_and_another_key_is_refused() {
    let directory = tempfile::tempdir().expect("a directory");
    {
        let connection = store(directory.path(), "postio.db");
        connection
            .execute_batch(
                "CREATE TABLE secrets(word TEXT); INSERT INTO secrets VALUES ('rhubarb')",
            )
            .expect("write");
    }

    let raw = std::fs::read(directory.path().join("postio.db")).expect("the file");
    assert!(
        !raw.starts_with(b"SQLite format 3"),
        "the store is plaintext"
    );
    assert!(
        !raw.windows(7).any(|w| w == b"rhubarb"),
        "the row is readable in the raw file"
    );

    let wrong = Connection::open(
        directory.path().join("postio.db").to_str().expect("utf-8"),
        Some(Encryption {
            cipher: "aes256gcm".to_owned(),
            hexkey: "ff".repeat(32),
        }),
    );
    let refused = match wrong {
        Err(_) => true,
        Ok(connection) => connection
            .query_row("SELECT word FROM secrets", (), |row| {
                row.get::<_, String>(0)
            })
            .is_err(),
    };
    assert!(refused, "another key opened the store");
}
