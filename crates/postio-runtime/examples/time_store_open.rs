//! How long opening a store takes, and how much of that is the WAL (#1175).
//!
//! The startup budget is under 500 ms (`docs/PERFORMANCE.md`) and the live
//! install takes about five seconds to show a window. `Phase::Store` is the
//! one blocking-I/O phase before the main loop starts, and the store it opens
//! has a 676 MB write-ahead log beside an 868 MB database. This measures
//! whether those two facts are the same fact.
//!
//! It opens the database the way the application does — `Database::open`,
//! read-write, the same pragmas — because a read-only open does not recover
//! a WAL and so cannot answer the question.
//!
//! # Point it at a copy, never at the live store
//!
//! Opening read-write checkpoints, which would destroy the very state being
//! measured and take the answer with it. On btrfs a copy is free and
//! instant:
//!
//! ```sh
//! mkdir -p ~/src/postio-wal-probe && cd ~/.local/share/postio
//! cp --reflink=always postio.db postio.db-wal postio.db-shm ~/src/postio-wal-probe/
//! cargo run -p postio-runtime --example time_store_open -- ~/src/postio-wal-probe/postio.db
//! ```
//!
//! It refuses to run against the default store path for that reason.
//!
//! # What it prints
//!
//! Three opens. The first pays whatever the WAL costs; then the WAL is
//! checkpointed with `TRUNCATE`, and the next two show what an open costs
//! without one. If the first is slow and the rest are fast, the WAL is the
//! startup cost and bounding it is the fix. If all three are the same, the
//! five seconds are somewhere else and this issue is looking in the wrong
//! place.

use std::path::{Path, PathBuf};
use std::time::Instant;

use postio_account::secret::{AccountKey, KeyringSecretStore, SecretStore};
use postio_storage::Database;
use postio_storage::key::{STORE_KEY_ENTRY, StoreKey};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => return Err("pass the path to a *copy* of a store; see the module docs".into()),
    };
    let live = std::env::var("HOME")
        .map(|home| Path::new(&home).join(".local/share/postio/postio.db"))
        .unwrap_or_default();
    if path == live {
        return Err(
            "refusing to open the live store: this opens read-write, which \
             checkpoints the WAL and destroys what is being measured. Copy it \
             first -- `cp --reflink=always` is instant on btrfs."
                .into(),
        );
    }

    let secrets = KeyringSecretStore::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // The other half of `Phase::Store`, which its own doc comment calls out:
    // "a D-Bus round trip to the keyring and SQLCipher's key derivation, both
    // before the main loop starts". Timed separately because only one of the
    // two is the database.
    let keyring_started = Instant::now();
    let stored = runtime.block_on(secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)))?;
    let keyring = keyring_started.elapsed().as_secs_f64() * 1000.0;
    println!("keyring round trip:        {keyring:>8.0} ms");
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?;

    println!("store: {}", path.display());
    report_sizes(&path);

    // ── the open the application pays for ────────────────────────────────
    let first = time_open(&path, &key)?;
    println!("\nopen #1 (as found):        {first:>8.0} ms");
    report_sizes(&path);

    // ── take the WAL out of the picture ──────────────────────────────────
    {
        let database = Database::open(&path, &key.derive(postio_storage::key::Purpose::Database))?;
        let connection = database.connection()?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    println!("\nafter wal_checkpoint(TRUNCATE):");
    report_sizes(&path);

    let second = time_open(&path, &key)?;
    let third = time_open(&path, &key)?;
    println!("\nopen #2 (WAL truncated):   {second:>8.0} ms");
    println!("open #3 (WAL truncated):   {third:>8.0} ms");

    let saved = first - second.max(third);
    println!(
        "\nthe WAL accounts for {saved:.0} ms of the first open ({:.0}%)",
        if first > 0.0 {
            saved / first * 100.0
        } else {
            0.0
        }
    );
    Ok(())
}

fn time_open(path: &Path, key: &StoreKey) -> Result<f64, Box<dyn std::error::Error>> {
    let subkey = key.derive(postio_storage::key::Purpose::Database);
    let started = Instant::now();
    let database = Database::open(path, &subkey)?;
    // One real read, because an open that has not touched a page has not
    // paid for the WAL index the way the application's first query does.
    let connection = database.connection()?;
    let _: i64 = connection.query_row("SELECT count(*) FROM mailboxes", [], |row| row.get(0))?;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    drop(connection);
    drop(database);
    Ok(elapsed)
}

fn report_sizes(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let candidate = PathBuf::from(name);
        match std::fs::metadata(&candidate) {
            Ok(meta) => println!(
                "  {:<10} {:>10.1} MB",
                candidate
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                meta.len() as f64 / 1_048_576.0
            ),
            Err(_) => continue,
        }
    }
}
