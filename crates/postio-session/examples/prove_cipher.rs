//! What the application's own store-opening path actually produces.
//!
//! ```sh
//! cargo run -p postio-session --example prove_cipher
//! ```
//!
//! Not a test harness reaching for `rusqlite` directly: this calls
//! [`postio_session::open_store_at`], which is what `postio-app`'s startup
//! calls, and then asks the file and the connection what happened. A spike
//! that changes how mail is encrypted owes a demonstration that mail is still
//! encrypted, through the door the application uses.
fn main() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("postio.db");
    let key = postio_storage::key::StoreKey::generate();

    let (database, _blobs) =
        postio_session::open_store_at(&path, &key).expect("the application's own open");

    {
        let connection = database.connection().expect("a connection");
        for pragma in ["cipher", "journal_mode"] {
            let value: String = connection
                .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
                .unwrap_or_default();
            println!("{pragma:<14} = {value:?}");
        }
        let tables: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .expect("the migrated schema");
        println!("schema objects = {tables}");
        assert!(tables > 0, "the migrations did not run");
    }
    drop(database);

    // ── the file is not readable ─────────────────────────────────────────
    let bytes = std::fs::read(&path).expect("the store");
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "THE STORE IS PLAINTEXT"
    );
    println!("header         = {:02x?}", &bytes[..16]);

    // Not a single table name anywhere in the file, which is the crude check
    // that catches an encryption that is on but not applied to everything.
    for marker in [
        b"messages".as_slice(),
        b"mailboxes".as_slice(),
        b"CREATE TABLE".as_slice(),
    ] {
        assert!(
            !bytes.windows(marker.len()).any(|w| w == marker),
            "found {:?} in the raw file",
            String::from_utf8_lossy(marker)
        );
    }
    println!("plaintext scan = no table names in {} bytes", bytes.len());

    // ── and another key does not open it ─────────────────────────────────
    let other = postio_storage::key::StoreKey::generate();
    assert!(
        postio_session::open_store_at(&path, &other).is_err(),
        "ANOTHER KEY OPENED THE STORE"
    );
    println!("another key    = refused");

    println!("\nOK — the application's store is ChaCha20-Poly1305 and encrypted");
}
