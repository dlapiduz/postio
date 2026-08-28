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

static MIGRATIONS: [Migration; 1] = [Migration {
    version: 1,
    name: "initial_schema",
    sql: include_str!("0001_initial_schema.sql"),
}];

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

    let mut applied = 0;
    for migration in migrations.iter().filter(|m| m.version > from) {
        apply(connection, migration)?;
        applied += 1;
    }

    let to = schema_version(connection)?;
    if applied > 0 {
        tracing::info!(from, to, applied, "applied schema migrations");
    } else {
        tracing::debug!(version = to, "schema is up to date");
    }

    Ok(MigrationReport { from, to, applied })
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
