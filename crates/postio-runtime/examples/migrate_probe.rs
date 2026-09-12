//! What the next migration launch costs: open a *copy* of a store and run
//! the shipped migrations against it, timed.
//!
//! Mutates its argument — point it at a copy, never at the live store. Runs
//! `migrate` twice so the second run prices the ordinary at-head no-op.
//!
//! ```sh
//! POSTIO_DIAG_KEY=$(secret-tool lookup application postio account "local store encryption key") \
//!   cargo run --release -p postio-runtime --example migrate_probe -- /path/to/copy/postio.db
//! ```

use std::time::Instant;

use postio_storage::key::{Purpose, StoreKey};
use rusqlite::Connection;

fn main() {
    let master = StoreKey::from_hex(std::env::var("POSTIO_DIAG_KEY").unwrap().trim()).unwrap();
    let key = master.derive(Purpose::Database);
    let path = std::env::args()
        .nth(1)
        .expect("pass the path to a COPY of a store");
    assert!(
        !path.contains(".local/share/postio"),
        "refusing the live store: this probe writes"
    );

    let t0 = Instant::now();
    let mut connection = Connection::open(&path).unwrap();
    postio_storage::db::configure(&connection, &key).expect("unlock");
    let opened = t0.elapsed();

    let t1 = Instant::now();
    let report = postio_storage::migrate(&mut connection).expect("migrate");
    let migrated = t1.elapsed();

    let t2 = Instant::now();
    let noop = postio_storage::migrate(&mut connection).expect("migrate again");
    let nooped = t2.elapsed();

    let t3 = Instant::now();
    let broken: usize = {
        let mut statement = connection.prepare("PRAGMA foreign_key_check").unwrap();
        statement.query_map([], |_| Ok(())).unwrap().count()
    };
    let checked = t3.elapsed();

    println!("open + key + pragmas: {opened:?}");
    println!("foreign_key_check alone ({broken} violations): {checked:?}");
    println!(
        "migrate {} -> {} ({} applied, incl. foreign_key_check): {migrated:?}",
        report.from, report.to, report.applied
    );
    println!(
        "migrate again at head ({} applied): {nooped:?}",
        noop.applied
    );
}
