//! Read-only: which pages of the store still authenticate (#1533).
//!
//! SQLCipher authenticates every page with an HMAC. A page that fails is not
//! a decryption that returns nonsense — it is a page the library will not
//! hand back, and on this build it **segfaults inside its own error path**
//! rather than returning an error:
//!
//! ```text
//! ERROR CORE sqlcipher_page_cipher: hmac check failed for pgno=29041
//! #0 sqlcipher_memset  #1 sqlite3Codec  #2 readDbPage
//!    ... ThreadRepository::count_of <- SqliteStore::read_thread_page
//! ```
//!
//! So the application dies when a read happens to touch the damaged page,
//! which presents as "this folder shows nothing" for whichever folders reach
//! it and as nothing at all for the rest.
//!
//! `PRAGMA cipher_integrity_check` walks every page and reports the ones that
//! do not authenticate, without writing anything. That is the difference
//! between "the store is damaged" and "the store is damaged *here, this
//! much*", which decides whether a resync is worth it or the file is done.
//!
//! Postio's store is a cache of the server (`CLAUDE.md`: no backwards
//! compatibility, rows are rebuilt or resynced), so the answer to real damage
//! is to rebuild rather than to repair. What cannot be rebuilt is anything
//! local-only: drafts that never sent, and the remote-image allow-list.
//!
//! ```sh
//! cargo run -p postio-runtime --example inspect_integrity
//! ```

use postio_account::secret::{AccountKey, KeyringSecretStore, SecretStore};
use postio_storage::key::{Purpose, STORE_KEY_ENTRY, StoreKey};
use rusqlite::{Connection, OpenFlags};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs_store_path();
    println!("store: {}", path.display());
    if !path.exists() {
        return Err(format!("no store at {}", path.display()).into());
    }

    let secrets = KeyringSecretStore::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let stored = runtime.block_on(secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)))?;
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?.derive(Purpose::Database);
    let connection = open_store(&path, &key)?;

    let pages: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    // As text, and under the column name `cipher_page_size`: SQLCipher
    // intercepts this pragma and does not answer it the way SQLite does.
    // Asking for an integer fails with `InvalidColumnType`, which is a
    // confusing way for an integrity checker to die before checking anything.
    let size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get::<_, String>(0))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(4096);
    println!(
        "pages: {pages} of {size} bytes ({} MB)\n",
        pages * size / 1_048_576
    );

    // Every page, reported one line per failure. It reads the whole file, so
    // it is slow on a large store and says nothing at all on a healthy one.
    println!("running PRAGMA cipher_integrity_check (reads every page) —");
    let mut statement = connection.prepare("PRAGMA cipher_integrity_check")?;
    let mut rows = statement.query([])?;
    let mut failures = Vec::new();
    while let Some(row) = rows.next()? {
        failures.push(row.get::<_, String>(0)?);
    }

    if failures.is_empty() {
        println!("  every page authenticates: the file is intact.");
    } else {
        println!("  {} page(s) do not authenticate:", failures.len());
        for line in failures.iter().take(40) {
            println!("    {line}");
        }
        if failures.len() > 40 {
            println!("    … and {} more", failures.len() - 40);
        }
        println!(
            "\n  {:.4}% of the file. Postio's store is a cache of the server, so\n  \
             the repair is a resync rather than a rescue -- but a draft that never\n  \
             sent, and the remote-image allow-list, are local-only and go with it.",
            failures.len() as f64 * 100.0 / pages as f64
        );
    }

    // Ordinary SQLite structure, which is a different question: a page can
    // authenticate and still hold a broken b-tree.
    println!("\nrunning PRAGMA integrity_check —");
    match connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
        Ok(answer) => println!("  {answer}"),
        Err(error) => println!("  could not complete: {error}"),
    }

    Ok(())
}

fn dirs_store_path() -> std::path::PathBuf {
    std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").expect("HOME");
            std::path::Path::new(&home).join(".local/share/postio/postio.db")
        })
}

/// Open the store read-only under `mac`, or say it is not the one.
///
/// **The MAC has to be named.** `PRAGMA cipher_hmac_algorithm` decides how
/// pages are authenticated and cannot be changed once one has been read, so a
/// reader that leaves it alone gets SQLCipher's default — SHA-512 — and a
/// store written under SHA-256 answers `hmac check failed for pgno=1` and
/// `file is not a database`. That is what this example did until it was
/// pointed at a real store: the key was right and the pages would not open.
///
/// `db.rs` calls the two `PageMac::Sha256` (what a new store gets) and
/// `PageMac::Sha512` (what older ones carry, read but never written), and
/// that type is `pub(crate)` — so the strings are spelled here and the caller
/// tries both rather than guessing.
fn open_under(
    path: &std::path::Path,
    key: &postio_storage::key::Subkey,
    mac: &str,
) -> Result<Connection, Box<dyn std::error::Error>> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.execute_batch("PRAGMA cipher_memory_security = OFF;")?;
    {
        let hex = key.to_hex();
        connection.execute_batch(&format!("PRAGMA key = \"x'{}'\";", *hex))?;
    }
    // After the key and before anything reads a page, which is what SQLCipher
    // requires of this one.
    connection.execute_batch(&format!("PRAGMA cipher_hmac_algorithm = {mac};"))?;
    connection.execute_batch("PRAGMA query_only = ON;")?;
    // The probe: `sqlite_schema` is page 1, so this is the cheapest read that
    // proves both the key and the MAC.
    connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(connection)
}

/// The store, opened under whichever MAC it was written with.
fn open_store(
    path: &std::path::Path,
    key: &postio_storage::key::Subkey,
) -> Result<Connection, Box<dyn std::error::Error>> {
    // Newest first: a store made by this build is SHA-256.
    match open_under(path, key, "HMAC_SHA256") {
        Ok(connection) => Ok(connection),
        Err(_) => open_under(path, key, "HMAC_SHA512").map_err(|error| {
            format!("the store opened under neither HMAC_SHA256 nor HMAC_SHA512: {error}").into()
        }),
    }
}
