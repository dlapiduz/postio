//! The database is SQLCipher (ADR 0014 Q1, #300).
//!
//! Page-level encryption below SQLite's own machinery, so FTS5, WAL, the
//! migrations and every repository work unchanged and the encryption is
//! invisible above `Database::open`. What these tests hold down is the part
//! that is *not* invisible:
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
use postio_storage::{Database, test_support};

/// A database subkey from a fixed master key, so a test can reopen a store.
fn key(seed: u8) -> Subkey {
    StoreKey::from_bytes([seed; 32]).derive(Purpose::Database)
}

/// Something distinctive enough that finding it in the file is unambiguous.
const SECRET_SUBJECT: &str = "Zarquon-Vindaloo-Quintessence";
const SECRET_BODY: &str = "The frobnicator arrives on Thursday, Grimswick.";

/// Writes a message carrying the two markers above, and answers the store path.
fn a_store_with_a_secret(directory: &std::path::Path, key: &Subkey) -> std::path::PathBuf {
    let path = directory.join("postio.db");
    let database = Database::open(&path, key).expect("open");
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let messages = MessageRepository::new(&connection);
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    message.subject = Some(SECRET_SUBJECT.to_owned());
    let id = messages.create(&mut message).expect("create");
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(SECRET_BODY.to_owned()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("store the body");

    // Fold the WAL back into the file, or the assertions below would be
    // reading a database whose newest pages are still in `postio.db-wal`.
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .expect("checkpoint");
    drop(connection);
    drop(database);
    path
}

#[test]
fn the_database_file_holds_no_plaintext() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(1));

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

#[test]
fn the_same_key_reopens_the_store_and_the_mail_is_there() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(2));

    let database = Database::open(&path, &key(2)).expect("reopen with the same key");
    let connection = database.connection().expect("checkout");
    let (id, subject): (i64, Option<String>) = connection
        .query_row("SELECT id, subject FROM messages", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("the message written before the store was closed");
    assert_eq!(subject.as_deref(), Some(SECRET_SUBJECT));
    assert_eq!(
        MessageRepository::new(&connection)
            .body(postio_model::MessageId::new(id))
            .expect("body")
            .expect("the row")
            .text
            .as_deref(),
        Some(SECRET_BODY),
        "a body round-trips through compression and page encryption together"
    );
}

#[test]
fn a_wrong_key_is_refused_in_words_rather_than_reported_as_corruption() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(3));

    let error = Database::open(&path, &key(4)).expect_err("a different key must not open it");
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

#[test]
fn a_wrong_key_never_destroys_what_it_could_not_read() {
    // The failure that would be unforgivable: a refused open that leaves the
    // store unopenable by the *right* key afterwards.
    let directory = tempfile::tempdir().expect("a directory");
    let path = a_store_with_a_secret(directory.path(), &key(5));

    Database::open(&path, &key(6)).expect_err("the wrong key");

    let database = Database::open(&path, &key(5)).expect("the right key still opens it");
    let connection = database.connection().expect("checkout");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "the mail survived a failed open");
}

#[test]
fn temp_store_is_memory_so_sorts_never_spill_plaintext_to_disk() {
    // ADR 0014's threat model closes SQLite's temp spill explicitly: an
    // encrypted database whose sort scratch lands on disk in the clear has
    // encrypted the wrong thing.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let pragmas = postio_storage::db::read_pragmas(&connection).expect("read the pragmas");
    assert_eq!(
        pragmas.temp_store, 2,
        "temp_store must be MEMORY (2), not FILE or DEFAULT"
    );
}

#[test]
fn the_whole_test_suite_runs_against_an_encrypted_store() {
    // `test_support` passes a fixed key, so nothing in the suite exercises a
    // plaintext configuration that no longer ships (ADR 0014 Q3). This asserts
    // the helper actually encrypts rather than merely opening.
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    message.subject = Some(SECRET_SUBJECT.to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .expect("checkpoint");
    drop(connection);

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
