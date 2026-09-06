//! Which MAC authenticates a page, and that an older store still opens (#1216).
//!
//! SQLCipher 4 defaults to HMAC-SHA512. On any CPU with the SHA extensions
//! that is the one part of the page path with no hardware behind it —
//! `sha_ni` accelerates SHA-1 and SHA-256, AES-NI accelerates the cipher, and
//! nothing accelerates SHA-512. A profile of a real 957 MiB mailbox put 45.9%
//! of its samples in `sha512_block_data_order_avx2` against 2.7% in
//! `aesni_cbc_encrypt`, and `hmac_cost.rs` measures the swap at 1.7x.
//!
//! The algorithm is written into the database, so this is a format choice and
//! not a tuning knob: a store made with one cannot be read with the other.
//! That is what the fallback below is for.

use postio_storage::Database;
use postio_storage::key::{Purpose, StoreKey};
use rusqlite::Connection;

const KEY_BYTE: u8 = 0x5a;

fn key() -> postio_storage::key::Subkey {
    StoreKey::from_bytes([KEY_BYTE; postio_storage::key::KEY_BYTES]).derive(Purpose::Database)
}

/// Open `path` by hand with `mac`, and say whether the pages verify.
fn opens_with(path: &std::path::Path, mac: &str) -> bool {
    let Ok(connection) = Connection::open(path) else {
        return false;
    };
    if connection
        .execute_batch("PRAGMA cipher_memory_security = OFF;")
        .is_err()
    {
        return false;
    }
    let hex = key().to_hex();
    if connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", *hex))
        .is_err()
    {
        return false;
    }
    drop(hex);
    if connection
        .execute_batch(&format!("PRAGMA cipher_hmac_algorithm = {mac};"))
        .is_err()
    {
        return false;
    }
    // The same probe `db::configure` uses: page 1 either verifies or it does
    // not, and a wrong MAC fails exactly like a wrong key.
    connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .is_ok()
}

/// A store built by hand with `mac`, holding one table.
fn make_with(path: &std::path::Path, mac: &str) {
    let connection = Connection::open(path).expect("open");
    connection
        .execute_batch("PRAGMA cipher_memory_security = OFF;")
        .expect("memory security");
    let hex = key().to_hex();
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", *hex))
        .expect("key");
    drop(hex);
    connection
        .execute_batch(&format!("PRAGMA cipher_hmac_algorithm = {mac};"))
        .expect("mac");
    connection
        .execute_batch("CREATE TABLE relic (id INTEGER PRIMARY KEY, note TEXT);")
        .expect("schema");
    connection
        .execute(
            "INSERT INTO relic (note) VALUES ('written under the old MAC')",
            [],
        )
        .expect("insert");
}

#[test]
fn a_new_store_authenticates_its_pages_with_sha256() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("store.db");
    {
        let database = Database::open(&path, &key()).expect("a new store");
        let connection = database.connection().expect("a connection");
        connection
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY);")
            .expect("schema");
    }

    assert!(
        opens_with(&path, "HMAC_SHA256"),
        "a store this version creates must be authenticated with HMAC_SHA256 -- \
         the MAC is 1.7x of the page-read path on hardware with the SHA \
         extensions (`hmac_cost.rs`)"
    );
    assert!(
        !opens_with(&path, "HMAC_SHA512"),
        "and it must not also read as SHA512, which would mean the pragma was \
         not applied and this test proves nothing"
    );
}

#[test]
fn a_store_written_with_the_old_mac_says_so_rather_than_blaming_the_key() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("old.db");
    make_with(&path, "HMAC_SHA512");

    // Not opened, and deliberately: pre-v1 there is nothing deployed to
    // protect, and the store is a cache of the server rather than a record of
    // anything. What matters is that the refusal is *true*. A wrong key and an
    // older MAC fail identically -- neither verifies page 1 -- so without this
    // the user is told their store "belongs to another installation", which is
    // false, alarming, and gives them nothing to do.
    let error = Database::open(&path, &key()).expect_err("the old format cannot be read");
    assert!(
        matches!(error, postio_storage::Error::StorePredatesPageMac),
        "expected the store to be recognised as pre-MAC-change, got {error:?}"
    );
    let said = error.to_string();
    assert!(
        said.contains("resync") && said.contains("no mail has been lost"),
        "the message has to say what to do and that nothing is lost: {said}"
    );
}

#[test]
fn a_wrong_key_is_still_a_wrong_key() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("store.db");
    {
        let _ = Database::open(&path, &key()).expect("a new store");
    }
    // The probe that recognises an older store must not soften a wrong key
    // into something reassuring: a key that opens nothing is still a wrong
    // key, and says so.
    let other =
        StoreKey::from_bytes([0x11; postio_storage::key::KEY_BYTES]).derive(Purpose::Database);
    let error = Database::open(&path, &other).expect_err("a wrong key opens nothing");
    assert!(
        matches!(error, postio_storage::Error::WrongStoreKey),
        "a wrong key must still read as a wrong key, not as a stale format: {error:?}"
    );
}
