//! The database is encrypted (ADR 0014 Q1, #300).
//!
//! Page-level encryption below the engine's own machinery, so the search
//! index, the WAL and every repository work unchanged and the encryption is
//! invisible above `Store::open`. It was SQLCipher's AES-256-CBC plus an
//! HMAC; it is the engine's own AES-256-GCM now, and what these tests hold
//! down did not change with it -- which is the point of their being about
//! properties rather than about a cipher:
//!
//! * **The bytes on disk are ciphertext.** Since ADR 0020 message bodies are
//!   rows, so this file is now what stands between a stolen laptop and the
//!   full text of every message. A test that only checked "it opens" would
//!   pass just as well against a plaintext database.
//! * **A wrong key is refused, in words.** Not a panic, not an empty mailbox,
//!   and above all not a store that opens and then reports corruption later.
//! * **There is no plaintext fallback.** Every constructor takes a key; there
//!   is nothing to call that would open an unencrypted store.

use postio_model::{BodyState, Message};
use postio_storage::key::{Purpose, StoreKey, Subkey};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::{Store, test_support};

/// A database subkey from a fixed master key, so a test can reopen a store.
fn key(seed: u8) -> Subkey {
    StoreKey::from_bytes([seed; 32]).derive(Purpose::Database)
}

/// Something distinctive enough that finding it in the file is unambiguous.
const SECRET_SUBJECT: &str = "Zarquon-Vindaloo-Quintessence";
const SECRET_BODY: &str = "The frobnicator arrives on Thursday, Grimswick.";

/// Writes a message carrying the two markers above, and answers the store path.
async fn a_store_with_a_secret(directory: &std::path::Path, key: &Subkey) -> std::path::PathBuf {
    let path = directory.join("postio.db");
    let database = Store::open(&path, key).await.expect("open");
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let messages = MessageRepository::new(&connection);
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    message.subject = Some(SECRET_SUBJECT.to_owned());
    let id = messages.create(&mut message).await.expect("create");
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(SECRET_BODY.to_owned()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .await
        .expect("store the body");

    // Fold the WAL back into the file, or the assertions below would be
    // reading a database whose newest pages are still in `postio.db-wal`.
    drop(connection);
    database.truncate_log().await.expect("checkpoint");
    drop(database);
    path
}

#[tokio::test]
async fn the_database_file_holds_no_plaintext() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(1)).await;

    let bytes = std::fs::read(&path).expect("read the database file");
    assert!(!bytes.is_empty(), "nothing was written");

    // The subject is TEXT in a row; the body is a compressed BLOB. Neither may
    // be findable in the file, and the subject is the one that would be if
    // encryption were off — ADR 0020's compression is not a privacy mechanism
    // and must not be mistaken for one.
    assert!(
        !contains(&bytes, SECRET_SUBJECT.as_bytes()),
        "the subject is sitting in the file in the clear"
    );
    assert!(
        !contains(&bytes, SECRET_BODY.as_bytes()),
        "the body text is sitting in the file in the clear"
    );

    // And the file is not a plain SQLite database at all: an unencrypted one
    // starts with this, and SQLCipher encrypts page 1 including the header.
    assert!(
        !bytes.starts_with(b"SQLite format 3\0"),
        "the file announces itself as an unencrypted SQLite database"
    );
}

#[tokio::test]
async fn the_same_key_reopens_the_store_and_the_mail_is_there() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(2)).await;

    let database = Store::open(&path, &key(2))
        .await
        .expect("reopen with the same key");
    let connection = database.connect().await.expect("checkout");
    let (id, subject): (i64, Option<String>) =
        postio_storage::sql::one(&connection, "SELECT id, subject FROM messages", (), |row| {
            Ok((
                postio_storage::sql::RowExt::col(row, 0)?,
                postio_storage::sql::RowExt::col(row, 1)?,
            ))
        })
        .await
        .expect("the message written before the store was closed");
    assert_eq!(subject.as_deref(), Some(SECRET_SUBJECT));
    assert_eq!(
        MessageRepository::new(&connection)
            .body(postio_model::MessageId::new(id))
            .await
            .expect("body")
            .expect("the row")
            .text
            .as_deref(),
        Some(SECRET_BODY),
        "a body round-trips through compression and page encryption together"
    );
}

#[tokio::test]
async fn a_wrong_key_is_refused_in_words_rather_than_reported_as_corruption() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(3)).await;

    let error = Store::open(&path, &key(4))
        .await
        .expect_err("a different key must not open it");
    let said = error.to_string();

    // The sentence reaches a person: `postio_session::open_store_at` puts it
    // on screen (#404), and "file is not a database" tells them their mail is
    // corrupt when it is intact and simply locked.
    assert!(
        !said.to_ascii_lowercase().contains("not a database"),
        "the raw SQLite wording leaks to the user: {said}"
    );
    assert!(
        said.to_ascii_lowercase().contains("key"),
        "the message must say what is actually wrong: {said}"
    );
}

#[tokio::test]
async fn a_wrong_key_never_destroys_what_it_could_not_read() {
    // The failure that would be unforgivable: a refused open that leaves the
    // store unopenable by the *right* key afterwards.
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(5)).await;

    Store::open(&path, &key(6))
        .await
        .expect_err("the wrong key");

    let database = Store::open(&path, &key(5))
        .await
        .expect("the right key still opens it");
    let connection = database.connect().await.expect("checkout");
    let count: i64 =
        postio_storage::sql::one(&connection, "SELECT count(*) FROM messages", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("count");
    assert_eq!(count, 1, "the mail survived a failed open");
}

#[tokio::test]
async fn temp_store_is_memory_so_sorts_never_spill_plaintext_to_disk() {
    // ADR 0014's threat model closes SQLite's temp spill explicitly: an
    // encrypted database whose sort scratch lands on disk in the clear has
    // encrypted the wrong thing.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    // Asked of the connection rather than of a struct this crate fills in:
    // there is no pragma-reading helper any more, and asking the engine is
    // the stronger question anyway -- it answers what is actually set.
    let temp_store: i64 = postio_storage::sql::one(&connection, "PRAGMA temp_store", (), |row| {
        postio_storage::sql::RowExt::col(row, 0)
    })
    .await
    .expect("read the pragma");
    assert_eq!(
        temp_store, 2,
        "temp_store must be MEMORY (2), not FILE or DEFAULT. The engine \
         defaults it to 0, so `Store::connect` sets it on every connection."
    );
}

#[tokio::test]
async fn the_whole_test_suite_runs_against_an_encrypted_store() {
    // `test_support` passes a fixed key, so nothing in the suite exercises a
    // plaintext configuration that no longer ships (ADR 0014 Q3). This asserts
    // the helper actually encrypts rather than merely opening.
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    message.subject = Some(SECRET_SUBJECT.to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create");
    drop(connection);
    database.truncate_log().await.expect("checkpoint");

    let bytes = std::fs::read(database.directory().join("postio.db")).expect("read");
    assert!(
        !contains(&bytes, SECRET_SUBJECT.as_bytes()),
        "the test helper opened a plaintext store"
    );
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
