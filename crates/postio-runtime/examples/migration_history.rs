//! Read-only: when each schema migration was applied, and how long it took.
//!
//! `apply` stamps `applied_at` (milliseconds) inside each migration's own
//! transaction, so the gap between consecutive rows *within one run* is the
//! later migration's duration — the first row of a run has no baseline and
//! prints only its clock time. Runs are split on gaps longer than ten
//! minutes, which no migration has yet earned.
//!
//! Prints versions, names and timestamps only — never message content,
//! never the key.
//!
//! ```sh
//! POSTIO_DIAG_KEY=$(secret-tool lookup application postio account "local store encryption key") \
//!   cargo run --release -p postio-runtime --example migration_history -- /path/to/copy/postio.db
//! ```

use postio_storage::key::{Purpose, StoreKey};
use rusqlite::{Connection, OpenFlags};

fn main() {
    let master = StoreKey::from_hex(std::env::var("POSTIO_DIAG_KEY").unwrap().trim()).unwrap();
    let key = master.derive(Purpose::Database);
    let path = std::env::args()
        .nth(1)
        .expect("pass the path to a copy of a store");
    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    postio_storage::db::configure(&c, &key).expect("unlock");

    let mut statement = c
        .prepare("SELECT version, name, applied_at FROM schema_migrations ORDER BY version")
        .unwrap();
    let rows: Vec<(u32, String, i64)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    const NEW_RUN_GAP_MS: i64 = 10 * 60 * 1000;
    let mut previous: Option<i64> = None;
    for (version, name, applied_at) in rows {
        let stamp = chrono::DateTime::from_timestamp_millis(applied_at)
            .map(|utc| {
                utc.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S%.3f")
                    .to_string()
            })
            .unwrap_or_else(|| format!("{applied_at} ms"));
        match previous {
            Some(before) if applied_at - before < NEW_RUN_GAP_MS => {
                println!(
                    "{version:>4}  {stamp}  +{:>8} ms  {name}",
                    applied_at - before
                );
            }
            _ => println!("{version:>4}  {stamp}  {:>12}  {name}", "run start"),
        }
        previous = Some(applied_at);
    }
}
