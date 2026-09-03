//! Forward-only, numbered schema migrations.
//!
//! # There is one, and that is deliberate
//!
//! `0001_initial_schema.sql` is the whole schema, not the first of a series of
//! corrections. It replaced twenty-five numbered migrations in August 2026 —
//! every one of them a fact about a past mistake and none of them a fact about
//! the schema, so a reader had to walk all twenty-five to find out what was
//! true.
//!
//! Collapsing them discarded every existing store, which was affordable
//! exactly once, while the installed user count was one. ADR 0020 records the
//! decision and calls it a licence rather than a policy. **It does not
//! generalize**: the rules below apply again from here, and the next schema
//! change is `0002`.
//!
//! # The rules
//!
//! 1. **Migrations are numbered from 1 with no gaps** and are applied in
//!    ascending order. [`migrate`] refuses a list that is not.
//! 2. **A migration that has been applied is immutable.** Its text is
//!    checksummed when it is applied and re-checked on every open; editing one
//!    after release is [`Error::MigrationChanged`], not a silent divergence
//!    between the developer's database and the user's.
//! 3. **There is no down migration.** Fixing a mistake means adding a new
//!    numbered migration.
//! 4. **Each migration runs inside its own transaction**, together with the
//!    bookkeeping row that records it. A migration that fails leaves the
//!    database exactly as it was before that migration started.
//! 5. **Running to head twice is a no-op**, so [`migrate`] is safe to call on
//!    every start.
//!
//! # Adding a migration
//!
//! Drop `NNNN_name.sql` next to this file and append a [`Migration`] entry to
//! [`all`]. Never touch an existing entry or its file.
//!
//! ```no_run
//! # use rusqlite::Connection;
//! # fn main() -> Result<(), postio_storage::Error> {
//! let mut connection = Connection::open("postio.db")?;
//! postio_storage::migrate(&mut connection)?;
//! # Ok(())
//! # }
//! ```

use rusqlite::Connection;

use crate::error::{Error, Result};

/// One numbered, forward-only schema change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    /// Position in the sequence. The first migration is `1`.
    pub version: u32,
    /// A short human name, recorded alongside the version. Descriptive only.
    pub name: &'static str,
    /// The SQL to execute. May contain several statements.
    pub sql: &'static str,
}

impl Migration {
    /// A stable digest of [`Self::sql`], used to detect an edited migration.
    ///
    /// FNV-1a: not cryptographic, and does not need to be — this guards against
    /// an honest mistake by a developer, not against an attacker who already
    /// has write access to both the binary and the database.
    fn checksum(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in self.sql.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

/// Every migration this build knows, in ascending version order.
pub fn all() -> &'static [Migration] {
    &MIGRATIONS
}

/// The schema version a fully migrated database is at.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}

static MIGRATIONS: [Migration; 7] = [
    Migration {
        version: 1,
        name: "initial_schema",
        sql: include_str!("0001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        name: "text_is_flowed",
        sql: include_str!("0002_text_is_flowed.sql"),
    },
    Migration {
        version: 3,
        name: "draft_message_id",
        sql: include_str!("0003_draft_message_id.sql"),
    },
    Migration {
        version: 4,
        name: "draft_unconfirmed",
        sql: include_str!("0004_draft_unconfirmed.sql"),
    },
    Migration {
        version: 5,
        name: "list_indexes_cover_their_filters",
        sql: include_str!("0005_list_indexes_cover_their_filters.sql"),
    },
    Migration {
        version: 6,
        name: "body_headers_truncated",
        sql: include_str!("0006_body_headers_truncated.sql"),
    },
    Migration {
        version: 7,
        name: "body_encoding_problems",
        sql: include_str!("0007_body_encoding_problems.sql"),
    },
];

/// What [`migrate`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    /// The schema version before the run.
    pub from: u32,
    /// The schema version after the run.
    pub to: u32,
    /// How many migrations were applied. `0` means the database was at head.
    pub applied: usize,
}

impl MigrationReport {
    /// Whether the database was already at head and nothing ran.
    pub fn is_no_op(&self) -> bool {
        self.applied == 0
    }
}

/// The bookkeeping table. Created before anything else and never altered by a
/// later migration.
const SCHEMA_MIGRATIONS: &str = "\
CREATE TABLE IF NOT EXISTS schema_migrations (
    version     INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL,
    checksum    TEXT    NOT NULL,
    applied_at  INTEGER NOT NULL
)";

/// The version this database is at, or `0` if nothing has been applied.
///
/// Cheap: safe to call on every start before deciding whether to migrate.
pub fn schema_version(connection: &Connection) -> Result<u32> {
    if !has_bookkeeping_table(connection)? {
        return Ok(0);
    }
    let version = connection.query_row(
        "SELECT coalesce(max(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

fn has_bookkeeping_table(connection: &Connection) -> Result<bool> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Brings `connection` up to the latest schema version.
///
/// Applies every migration the database has not seen, each in its own
/// transaction, in ascending order. Running against a database already at head
/// is a no-op that touches no data.
///
/// # Errors
///
/// * [`Error::SchemaTooNew`] if the database was written by a newer build. The
///   caller must not proceed: this build does not know what those columns mean.
/// * [`Error::MigrationChanged`] if an already-applied migration's SQL has been
///   edited.
/// * [`Error::Migration`] if a migration's SQL fails. That migration is rolled
///   back whole; the database stays at the last version that committed.
pub fn migrate(connection: &mut Connection) -> Result<MigrationReport> {
    migrate_with(connection, all())
}

/// [`migrate`], against an explicit migration list.
///
/// Exists so the runner's own behaviour — ordering, rollback, checksums — can
/// be tested without inventing real schema changes. Production code calls
/// [`migrate`].
pub fn migrate_with(
    connection: &mut Connection,
    migrations: &[Migration],
) -> Result<MigrationReport> {
    check_ordering(migrations)?;
    connection.execute_batch(SCHEMA_MIGRATIONS)?;

    let from = schema_version(connection)?;
    let known = migrations.last().map_or(0, |migration| migration.version);
    if from > known {
        return Err(Error::SchemaTooNew { found: from, known });
    }

    verify_applied(connection, migrations)?;

    // Step 1 of SQLite's own table-rebuild procedure, and not optional the
    // moment a migration rebuilds a table: a `DROP TABLE` prepared with
    // foreign keys enabled performs an implicit `DELETE FROM`, and that
    // *does* fire `ON DELETE CASCADE`. 0004 rebuilds `drafts`, which
    // `recipients` and `attachments` both cascade from -- so with the pragma
    // on it would take every draft's addresses and attached files with it,
    // report success, and leave a schema that looks perfectly correct.
    //
    // Out here rather than in `apply` because `PRAGMA foreign_keys` is
    // silently ignored inside a transaction, and `apply` runs each migration
    // in one. Restored below whether the run succeeded or not: the pragma is
    // per-connection, and handing back a connection with referential
    // integrity switched off would let every later write break it.
    //
    // 0004's own header says foreign keys are "deferred for the swap rather
    // than disabled". That is wrong twice — nothing defers them, and
    // `defer_foreign_keys` would not have helped, since it defers constraint
    // *checking* to commit and does not suppress `CASCADE` actions. The file
    // is left as it is on purpose: `verify_applied` checksums the SQL, so
    // editing even a comment would make every database that has already run
    // it refuse to open. This is the correction, and it belongs here anyway.
    let enforcing = scalar_pragma(connection, "foreign_keys")? != 0;
    if enforcing {
        connection.pragma_update(None, "foreign_keys", false)?;
    }

    let result = apply_all(connection, migrations, from);

    if enforcing {
        connection.pragma_update(None, "foreign_keys", true)?;
    }
    let applied = result?;

    let to = schema_version(connection)?;
    if applied > 0 {
        tracing::info!(from, to, applied, "applied schema migrations");
    } else {
        tracing::debug!(version = to, "schema is up to date");
    }

    Ok(MigrationReport { from, to, applied })
}

/// Reads a one-value pragma.
fn scalar_pragma(connection: &Connection, pragma: &str) -> Result<i64> {
    Ok(connection.query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))?)
}

/// Applies every migration past `from`, and proves the result still hangs
/// together.
///
/// Split out so the caller can restore `foreign_keys` on the way out of both
/// the success and the failure path.
fn apply_all(connection: &mut Connection, migrations: &[Migration], from: u32) -> Result<usize> {
    let mut applied = 0;
    for migration in migrations.iter().filter(|m| m.version > from) {
        apply(connection, migration)?;
        applied += 1;
    }

    // Step 12 of the rebuild procedure. Enforcement was off for the run, so
    // nothing above raised a constraint failure; this is what turns a
    // migration that broke a reference into a loud failure instead of a
    // database that is quietly wrong. Only when something was applied — on
    // the ordinary "already at head" path there is nothing to check and the
    // scan is not free.
    if applied > 0 {
        let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
        let broken: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        if let Some((table, parent)) = broken.first() {
            return Err(Error::MigrationBrokeReferences {
                table: table.clone(),
                parent: parent.clone(),
                rows: broken.len(),
            });
        }
    }

    Ok(applied)
}

/// Applies one migration and records it, atomically.
fn apply(connection: &mut Connection, migration: &Migration) -> Result<()> {
    let transaction = connection.transaction()?;

    transaction
        .execute_batch(migration.sql)
        .map_err(|source| Error::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;

    transaction
        .execute(
            "INSERT INTO schema_migrations (version, name, checksum, applied_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                migration.version,
                migration.name,
                migration.checksum(),
                now_millis(),
            ],
        )
        .map_err(|source| Error::Migration {
            version: migration.version,
            name: migration.name,
            source,
        })?;

    // Mirrored into the header so a tool that only has the file — a backup
    // check, `sqlite3 .dbinfo` — can see the version without a query. Written
    // inside the transaction, so it can never disagree with the table.
    transaction.pragma_update(None, "user_version", migration.version)?;

    transaction.commit()?;
    Ok(())
}

/// Rejects a list that is not numbered 1..n in ascending order.
fn check_ordering(migrations: &[Migration]) -> Result<()> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected = index as u32 + 1;
        if migration.version != expected {
            return Err(Error::MigrationOrder {
                position: index,
                expected,
                found: migration.version,
            });
        }
    }
    Ok(())
}

/// Confirms that every migration already recorded still has the SQL it had when
/// it ran. Forward-only means an applied migration is history, not source.
fn verify_applied(connection: &Connection, migrations: &[Migration]) -> Result<()> {
    let mut statement =
        connection.prepare("SELECT version, name, checksum FROM schema_migrations")?;
    let recorded = statement.query_map([], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in recorded {
        let (version, name, checksum) = row?;
        let Some(migration) = migrations.iter().find(|m| m.version == version) else {
            // A version this build has never heard of. `SchemaTooNew` already
            // covered the "newer database" case; a hole below head means the
            // list itself was edited.
            return Err(Error::MigrationChanged {
                version,
                name,
                detail: "no migration with this version exists in this build",
            });
        };
        if migration.checksum() != checksum {
            return Err(Error::MigrationChanged {
                version,
                name,
                detail: "its SQL has changed since it was applied; \
                         migrations are forward-only, add a new one instead",
            });
        }
    }
    Ok(())
}

/// Milliseconds since the Unix epoch.
///
/// `std` rather than `chrono`: the runner is the one place that must work
/// before any of the model's types are involved.
fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_migrations_are_numbered_from_one() {
        check_ordering(all()).expect("the shipped list must be well formed");
        assert_eq!(latest_version(), all().len() as u32);
    }

    /// #674 widened the `drafts` CHECK, and SQLite cannot do that in place:
    /// 0004 rebuilds the table. A rebuild is the one migration shape that can
    /// silently lose a user's unsent mail — a forgotten column, a positional
    /// `SELECT *`, a dropped index — so it is checked rather than reviewed.
    #[test]
    fn rebuilding_drafts_keeps_every_row_column_and_index() {
        let mut connection = Connection::open_in_memory().expect("sqlite");
        // Up to 0003: the schema as it stood before this rebuild.
        migrate_with(&mut connection, &all()[..3]).expect("migrate to 0003");
        connection
            .execute_batch(
                "INSERT INTO accounts (
                     id, display_name, address, incoming_host, incoming_port,
                     incoming_username, outgoing_host, outgoing_port,
                     outgoing_username, created_at
                 ) VALUES (
                     1, 'Ada', 'ada@example.com', 'imap.example.com', 993,
                     'ada', 'smtp.example.com', 587, 'ada', 0
                 );
                 INSERT INTO drafts (
                     id, account_id, kind, subject, body_text, state,
                     created_at, updated_at, rfc_message_id
                 ) VALUES (
                     7, 1, 'reply', 'the tide gate interlock', 'Looking now.',
                     'sending', 111, 222, '<reserved@example.com>'
                 );",
            )
            .expect("a draft mid-send, of the kind this migration exists for");

        migrate_with(&mut connection, all()).expect("migrate to head");

        let (account, kind, subject, body, state, created, updated, reserved): (
            i64,
            String,
            String,
            String,
            String,
            i64,
            i64,
            String,
        ) = connection
            .query_row(
                "SELECT account_id, kind, subject, body_text, state, created_at,
                        updated_at, rfc_message_id
                   FROM drafts WHERE id = 7",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .expect("the draft survived the rebuild");

        assert_eq!(account, 1);
        assert_eq!(kind, "reply");
        assert_eq!(subject, "the tide gate interlock");
        assert_eq!(body, "Looking now.");
        assert_eq!(state, "sending");
        assert_eq!((created, updated), (111, 222));
        assert_eq!(
            reserved, "<reserved@example.com>",
            "0003's reserved Message-ID has to survive, or every in-flight \
             retry becomes a second, distinct message (#461)"
        );

        // The new state is accepted, and a nonsense one still is not.
        connection
            .execute("UPDATE drafts SET state = 'unconfirmed' WHERE id = 7", [])
            .expect("the widened CHECK admits the new state");
        assert!(
            connection
                .execute("UPDATE drafts SET state = 'posted' WHERE id = 7", [])
                .is_err(),
            "the rebuild must not have dropped the CHECK altogether"
        );

        // And the indexes `DROP TABLE` took with it are back.
        let indexes: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'drafts'")
            .expect("a statement")
            .query_map([], |row| row.get(0))
            .expect("a query")
            .collect::<Result<_, _>>()
            .expect("index names");
        for expected in [
            "idx_drafts_account_updated",
            "idx_drafts_state",
            "idx_drafts_thread",
            "idx_drafts_message",
        ] {
            assert!(
                indexes.iter().any(|name| name == expected),
                "{expected} did not survive the rebuild: {indexes:?}"
            );
        }
    }

    #[test]
    fn checksums_depend_on_the_sql_and_not_on_the_name() {
        let a = Migration {
            version: 1,
            name: "a",
            sql: "SELECT 1;",
        };
        let renamed = Migration {
            name: "b",
            ..a.clone()
        };
        let edited = Migration {
            sql: "SELECT 2;",
            ..a.clone()
        };

        assert_eq!(a.checksum(), renamed.checksum());
        assert_ne!(a.checksum(), edited.checksum());
        assert_eq!(a.checksum().len(), 16, "a fixed-width hex digest");
    }

    #[test]
    fn out_of_order_lists_are_rejected() {
        let gap = [Migration {
            version: 2,
            name: "gap",
            sql: "",
        }];
        assert!(matches!(
            check_ordering(&gap),
            Err(Error::MigrationOrder {
                position: 0,
                expected: 1,
                found: 2
            })
        ));
    }

    #[test]
    fn an_empty_list_is_well_formed_and_at_version_zero() {
        check_ordering(&[]).expect("an empty list is trivially ordered");
        let mut connection = Connection::open_in_memory().expect("sqlite");
        let report = migrate_with(&mut connection, &[]).expect("migrate nothing");
        assert_eq!(
            report,
            MigrationReport {
                from: 0,
                to: 0,
                applied: 0
            }
        );
        assert!(report.is_no_op());
    }
}
