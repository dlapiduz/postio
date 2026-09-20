//! Read-only: does the store still read end to end (#1533).
//!
//! Under SQLCipher this ran `PRAGMA cipher_integrity_check`, because a page
//! that failed its HMAC was a segfault waiting inside whichever read touched
//! it first, and "damaged *here, this much*" decided between a resync and a
//! funeral. That pragma went with SQLCipher: this engine owns its encryption
//! and surfaces a page it cannot read as an error on the read
//! (`postio_storage`'s open path says what to do about a store this build
//! cannot read). What is left to ask is the structural question — do the
//! file's trees still hold together — and that is what this asks.
//!
//! Postio's store is a cache of the server (`CLAUDE.md`: no backwards
//! compatibility, rows are rebuilt or resynced), so the answer to real damage
//! is to rebuild rather than to repair. What cannot be rebuilt is anything
//! local-only: drafts that never sent, and the remote-image allow-list.
//!
//! **Point it at a copy of a store you care about** — the engine has no
//! read-only open, so nothing protects the live file from the engine's own
//! recovery on open except this instruction.
//!
//! ```sh
//! cargo run -p postio-runtime --example inspect_integrity
//! ```

use postio_account::secret::{AccountKey, KeyringSecretStore, SecretStore};
use postio_storage::key::{Purpose, STORE_KEY_ENTRY, StoreKey};
use postio_storage::sql::RowExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = dirs_store_path();
    println!("store: {}", path.display());
    if !path.exists() {
        return Err(format!("no store at {}", path.display()).into());
    }

    let secrets = KeyringSecretStore::default();
    let stored = secrets.retrieve(&AccountKey::new(STORE_KEY_ENTRY)).await?;
    if stored.is_empty() {
        return Err("the store key entry is empty; refusing to mint one".into());
    }
    let key = StoreKey::from_hex(stored.expose())?.derive(Purpose::Database);

    // The probe that used to be implicit in the MAC dance: page 1 read and
    // the schema counted, which proves the key fits and the file answers.
    let store = postio_storage::Store::open(&path, &key).await?;
    let connection = store.connect().await?;
    let tables =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM sqlite_schema", ()).await?;
    println!("schema objects: {tables}");

    // Structure, table by table: a count walks each table's tree end to end,
    // so a page the engine cannot read or a tree that lost a child surfaces
    // as an error naming the table rather than a crash in whichever folder
    // happened to reach it first.
    println!("\nreading every table end to end —");
    let names = postio_storage::sql::all(
        &connection,
        "SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name",
        (),
        |row| row.col::<String>(0),
    )
    .await?;
    let mut damaged = 0usize;
    for name in names {
        match postio_storage::sql::scalar(
            &connection,
            &format!("SELECT count(*) FROM \"{name}\""),
            (),
        )
        .await
        {
            Ok(rows) => println!("  {name:<28} {rows:>9} rows"),
            Err(error) => {
                damaged += 1;
                println!("  {name:<28} FAILED: {error}");
            }
        }
    }

    if damaged == 0 {
        println!("\nevery table reads end to end: the file is intact.");
    } else {
        println!(
            "\n{damaged} table(s) failed to read. Postio's store is a cache of the \
             server, so\nthe repair is a resync rather than a rescue -- but a draft that \
             never\nsent, and the remote-image allow-list, are local-only and go with it."
        );
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
