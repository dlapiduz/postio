//! A store an earlier build wrote is carried forward in place, not refused.
//!
//! Every file in `tests/schemas/` is a schema some build of Postio stamped a
//! store with -- that build's `schema::HEAD`, verbatim, named for its
//! fingerprint. Opening a store at any of them has to end where a fresh store
//! begins, with what it held still in it: the operation queue, drafts,
//! snoozes, reminders and the rest of what exists nowhere but this file.
//!
//! The refusal for a store *no* migration reaches is
//! `refuses_a_stale_schema.rs`; it stays a refusal, because a schema nobody
//! recorded cannot be carried anywhere.

use postio_storage::sql::{self, RowExt as _};
use postio_storage::{Store, schema, test_support};

/// Where the earlier schemas are kept.
const SCHEMAS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/schemas");

/// Every recorded schema: its file name and its text.
fn recorded() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = std::fs::read_dir(SCHEMAS)
        .expect("tests/schemas reads")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "sql"))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path()).expect("a schema reads"),
            )
        })
        .collect();
    found.sort();
    found
}

/// A store's shape, as facts two stores can be compared on: every object by
/// type and name, every table's columns, and every index and trigger by its
/// statement with the spacing and `IF NOT EXISTS` taken out (a migration
/// creates with it, a fresh store without).
async fn shape(store: &Store) -> Vec<String> {
    let connection = store.connect().await.expect("a connection");
    let objects = sql::all_unbounded(
        &connection,
        "SELECT type, name, coalesce(sql, '') FROM sqlite_schema
          WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        (),
        |row| Ok((row.text(0)?, row.text(1)?, row.text(2)?)),
    )
    .await
    .expect("the schema reads");
    let mut facts = Vec::new();
    for (kind, name, statement) in objects {
        if kind == "table" {
            let columns = sql::all_unbounded(
                &connection,
                &format!("PRAGMA table_info({name})"),
                (),
                |row| {
                    Ok(format!(
                        "{} {} notnull={} default={:?} pk={}",
                        row.text(1)?,
                        row.text(2)?,
                        row.int(3)?,
                        row.opt_text(4)?,
                        row.int(5)?,
                    ))
                },
            )
            .await
            .expect("the columns read");
            facts.push(format!("table {name}: {}", columns.join(", ")));
        } else {
            let statement = statement
                .replace("IF NOT EXISTS ", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            facts.push(format!("{kind} {name}: {statement}"));
        }
    }
    facts
}

/// What an earlier store held that is nowhere but here: an account, and a
/// change the person made that the server has not heard about yet.
const HELD: &str = "
INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                      incoming_username, outgoing_host, outgoing_port,
                      outgoing_username, created_at)
     VALUES (7, 'Ada', 'ada@example.com', 'imap.example.com', 993,
             'ada@example.com', 'smtp.example.com', 587, 'ada@example.com', 1);
INSERT INTO operation_queue (account_id, op_type, created_at, updated_at)
     VALUES (7, 'archive', 1, 1);
";

#[tokio::test]
async fn every_recorded_schema_is_migrated_in_place_to_head() {
    let schemas = recorded();
    assert!(
        !schemas.is_empty(),
        "tests/schemas holds the schemas earlier builds stamped; with none, \
         nothing here is tested"
    );
    let fresh_dir = tempfile::tempdir().unwrap();
    let fresh = Store::open(fresh_dir.path().join("postio.db"), &test_support::key())
        .await
        .expect("a fresh store opens");
    let head = shape(&fresh).await;

    for (file, text) in schemas {
        let stamp = schema::fingerprint(&text);
        assert_eq!(
            file,
            format!("{:08x}.sql", stamp as i32 as u32),
            "a recorded schema is named for the fingerprint its text hashes \
             to, so a migration's `from` finds it"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postio.db");
        Store::create_at_schema(&path, &test_support::key(), &text, HELD)
            .await
            .expect("a store at the earlier schema is written");

        let store = match Store::open(&path, &test_support::key()).await {
            Ok(store) => store,
            Err(error) => panic!(
                "a store stamped {file} has to open, migrated: an earlier \
                 build's store is the person's, and refusing it costs them \
                 everything only this file holds. Got: {error}"
            ),
        };
        assert_eq!(
            shape(&store).await,
            head,
            "a store migrated from {file} has to be shaped as a fresh one"
        );
        let connection = store.connect().await.expect("a connection");
        assert_eq!(
            sql::scalar(&connection, "PRAGMA user_version", ())
                .await
                .unwrap(),
            schema::FINGERPRINT,
            "a migrated store is stamped as this build's, or it migrates on \
             every open"
        );
        assert_eq!(
            sql::scalar(
                &connection,
                "SELECT count(*) FROM operation_queue WHERE account_id = 7",
                ()
            )
            .await
            .unwrap(),
            1,
            "the change the server has not heard about yet survives the \
             migration"
        );
        drop(connection);
        drop(store);
        Store::open(&path, &test_support::key())
            .await
            .expect("a migrated store opens again as this build's own");
    }
}
