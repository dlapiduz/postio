//! Turning a plaintext store into an encrypted one (ADR 0014 Q4, #301).
//!
//! These stores exist only in a handful of development checkouts, and their
//! contents are still somebody's mail. The claim the migration makes is
//! narrow and absolute — **a run that dies half-way loses nothing** — and the
//! only way to assert it is to stand at every point it could die, which
//! [`postio_storage::encrypt::Stage`] enumerates and
//! `encrypt::stopping_after` reaches.
//!
//! The rest is what "encrypted" has to mean afterwards: the database file is
//! not a SQLite file any more, the blobs are containers rather than bare
//! bytes, the ids the rows carry are the *new* keyed ones, and the plaintext
//! copy is gone rather than lying beside it.

use std::path::{Path, PathBuf};

use postio_model::{BlobId, Message};
use postio_storage::encrypt::{self, Outcome, Stage};
use postio_storage::key::{BlobKeys, Purpose, StoreKey};
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, Database};
use rusqlite::Connection;

/// Distinctive enough that finding it in a file is unambiguous.
const RAW_SOURCE: &[u8] =
    b"From: ada@example.com\r\nSubject: Zarquon\r\n\r\nThe frobnicator arrives Thursday.\r\n";
const PAYLOAD: &[u8] = b"%PDF-1.4 Grimswick-Vindaloo-Quintessence";

fn master(seed: u8) -> StoreKey {
    StoreKey::from_bytes([seed; 32])
}

/// A plaintext store: an unencrypted database and bare-bytes blobs, exactly
/// the shape a checkout from before ADR 0014 has on disk.
struct Plaintext {
    _directory: tempfile::TempDir,
    database: PathBuf,
    blobs: PathBuf,
    /// The unkeyed digests the rows point at before the migration runs.
    raw: String,
    payload: String,
}

impl Plaintext {
    fn build() -> Self {
        let directory = tempfile::tempdir().expect("a directory");
        let database = directory.path().join("postio.db");
        let blobs = directory.path().join("blobs");

        let mut connection = Connection::open(&database).expect("a plaintext database");
        postio_storage::migrate(&mut connection).expect("migrate");

        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let mut message = Message::new(account.id, inbox, chrono::Utc::now());
        message.subject = Some("Zarquon".to_owned());
        let message_id = MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create the message");

        let raw = write_bare_blob(&blobs, RAW_SOURCE);
        let payload = write_bare_blob(&blobs, PAYLOAD);

        connection
            .execute(
                "UPDATE messages SET raw_blob_id = ?1 WHERE id = ?2",
                rusqlite::params![raw, message_id.get()],
            )
            .expect("point the message at its source");
        connection
            .execute(
                "INSERT INTO attachments (message_id, mime_type, filename, size, blob_id)
                 VALUES (?1, 'application/pdf', 'invoice.pdf', ?2, ?3)",
                rusqlite::params![message_id.get(), PAYLOAD.len() as i64, payload],
            )
            .expect("point an attachment at its payload");
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .expect("checkpoint");
        drop(connection);

        Self {
            _directory: directory,
            database,
            blobs,
            raw,
            payload,
        }
    }

    fn enqueue_an_operation(&self) {
        let connection = Connection::open(&self.database).expect("open");
        connection
            .execute(
                "INSERT INTO operation_queue (account_id, op_type, created_at, updated_at)
                 VALUES ((SELECT id FROM accounts LIMIT 1), 'flag', 0, 0)",
                [],
            )
            .expect("enqueue");
    }
}

/// A blob as a store from before the container format wrote them: the bytes,
/// under the unkeyed digest, sharded two levels deep.
fn write_bare_blob(root: &Path, content: &[u8]) -> String {
    let digest = blake3::hash(content).to_hex().to_string();
    let directory = root.join(&digest[0..2]).join(&digest[2..4]);
    std::fs::create_dir_all(&directory).expect("shard directories");
    std::fs::write(directory.join(&digest[4..]), content).expect("write the blob");
    digest
}

/// Opens the migrated store and answers what it holds: the message's subject,
/// its raw source and its attachment payload, each read through the ids the
/// rows carry *now*.
fn read_everything(store: &Plaintext, master: &StoreKey) -> (String, Vec<u8>, Vec<u8>) {
    let database = Database::open(&store.database, &master.derive(Purpose::Database))
        .expect("the encrypted database opens");
    let connection = database.connection().expect("checkout");
    let (subject, raw, payload): (String, String, String) = connection
        .query_row(
            "SELECT m.subject, m.raw_blob_id, a.blob_id
               FROM messages m JOIN attachments a ON a.message_id = m.id",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the row survived");

    let blobs = BlobStore::open(&store.blobs, &BlobKeys::derive(master)).expect("the blob store");
    let raw = blobs.get(&BlobId::new(raw)).expect("the raw source");
    let payload = blobs.get(&BlobId::new(payload)).expect("the payload");
    (subject, raw, payload)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// What the migration leaves behind
// ---------------------------------------------------------------------------

#[test]
fn the_store_is_encrypted_afterwards_and_the_mail_is_all_there() {
    let store = Plaintext::build();
    let master = master(1);

    let outcome = encrypt::encrypt_store(&store.database, &master).expect("migrate");
    assert!(
        matches!(outcome, Outcome::Encrypted(report) if report.blobs == 2),
        "both blobs should have been re-encrypted, got {outcome:?}"
    );

    let (subject, raw, payload) = read_everything(&store, &master);
    assert_eq!(subject, "Zarquon");
    assert_eq!(raw, RAW_SOURCE);
    assert_eq!(payload, PAYLOAD);
}

#[test]
fn nothing_readable_is_left_on_disk() {
    let store = Plaintext::build();
    let master = master(2);
    encrypt::encrypt_store(&store.database, &master).expect("migrate");

    let database = std::fs::read(&store.database).expect("read the database file");
    assert!(
        !database.starts_with(b"SQLite format 3\0"),
        "the database still announces itself as an unencrypted SQLite file"
    );
    assert!(
        !contains(&database, b"Zarquon"),
        "the subject is still in the file in the clear"
    );

    for path in every_file(&store.blobs) {
        let bytes = std::fs::read(&path).expect("read a blob");
        assert!(
            !contains(&bytes, PAYLOAD) && !contains(&bytes, RAW_SOURCE),
            "{} still holds its plaintext",
            path.display()
        );
    }

    // And the plaintext copy is gone rather than sitting beside the encrypted
    // one, which would make the whole exercise decorative.
    let directory = store.database.parent().expect("a directory");
    for stray in std::fs::read_dir(directory)
        .expect("list the store")
        .flatten()
    {
        let name = stray.file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(".postio-"),
            "the migration left {name} behind"
        );
    }
}

#[test]
fn the_ids_are_re_keyed_rather_than_carried_over() {
    // Not cosmetic: an id that survived the migration would still be the plain
    // digest of the content, which is exactly the correlation ADR 0014 Q2
    // removed.
    let store = Plaintext::build();
    let master = master(3);
    encrypt::encrypt_store(&store.database, &master).expect("migrate");

    let database =
        Database::open(&store.database, &master.derive(Purpose::Database)).expect("open");
    let connection = database.connection().expect("checkout");
    let raw: String = connection
        .query_row("SELECT raw_blob_id FROM messages", [], |row| row.get(0))
        .expect("the row");
    assert_ne!(raw, store.raw, "the row still carries the unkeyed digest");
    assert_ne!(raw, store.payload);
}

#[test]
fn a_second_run_finds_nothing_to_do() {
    let store = Plaintext::build();
    let master = master(4);
    encrypt::encrypt_store(&store.database, &master).expect("migrate");

    assert_eq!(
        encrypt::encrypt_store(&store.database, &master).expect("second run"),
        Outcome::AlreadyEncrypted,
        "an encrypted store must not be migrated again"
    );
    let (subject, raw, _) = read_everything(&store, &master);
    assert_eq!(subject, "Zarquon");
    assert_eq!(raw, RAW_SOURCE, "the second run damaged the store");
}

#[test]
fn a_first_run_with_no_store_is_not_a_failure() {
    let directory = tempfile::tempdir().expect("a directory");
    assert_eq!(
        encrypt::encrypt_store(&directory.path().join("postio.db"), &master(5))
            .expect("no store is not an error"),
        Outcome::NoStore
    );
}

// ---------------------------------------------------------------------------
// Drain first
// ---------------------------------------------------------------------------

#[test]
fn a_store_with_work_still_queued_is_refused_and_left_alone() {
    let store = Plaintext::build();
    store.enqueue_an_operation();

    let error = encrypt::encrypt_store(&store.database, &master(6))
        .expect_err("an undrained queue must stop the migration");
    assert!(
        error.to_string().contains("server"),
        "the sentence must say what has to happen first: {error}"
    );

    // And it is still the plaintext store it was, rather than half of one.
    let database = std::fs::read(&store.database).expect("read");
    assert!(
        database.starts_with(b"SQLite format 3\0"),
        "a refused migration replaced the store anyway"
    );
    assert!(
        std::fs::read(
            store
                .blobs
                .join(&store.raw[0..2])
                .join(&store.raw[2..4])
                .join(&store.raw[4..])
        )
        .expect("the blob is where it was")
            == RAW_SOURCE
    );
}

// ---------------------------------------------------------------------------
// Acceptance: it can be killed anywhere
// ---------------------------------------------------------------------------

#[test]
fn a_migration_killed_at_any_point_loses_nothing() {
    // The claim ADR 0014 Q4's swap-last ordering exists for. Returning early
    // from a stage leaves exactly the on-disk state a killed process leaves:
    // nothing in `encrypt` publishes anything from a destructor, so there is
    // no cleanup a `SIGKILL` would have skipped.
    let stages = [
        Stage::Staged,
        Stage::Verified,
        Stage::DatabaseAside,
        Stage::OriginalsAside,
        Stage::DatabaseInPlace,
        Stage::Swapped,
    ];

    for (n, stage) in stages.into_iter().enumerate() {
        let store = Plaintext::build();
        let master = master(100 + n as u8);

        encrypt::stopping_after(&store.database, &master, stage).expect("the partial run");

        // What the next open does, and the only recovery there is.
        encrypt::encrypt_store(&store.database, &master)
            .unwrap_or_else(|error| panic!("the run after dying at {stage:?} failed: {error}"));

        let (subject, raw, payload) = read_everything(&store, &master);
        assert_eq!(subject, "Zarquon", "subject lost after dying at {stage:?}");
        assert_eq!(raw, RAW_SOURCE, "raw source lost after dying at {stage:?}");
        assert_eq!(payload, PAYLOAD, "payload lost after dying at {stage:?}");
    }
}

#[test]
fn a_store_killed_mid_swap_still_holds_every_byte_before_it_is_resumed() {
    // The stronger form: at the narrowest window -- both halves moved aside,
    // neither replacement in yet -- the mail is still on disk somewhere, which
    // is what "loses nothing" means before anything gets a chance to resume.
    let store = Plaintext::build();
    let master = master(200);
    encrypt::stopping_after(&store.database, &master, Stage::OriginalsAside).expect("partial run");

    let directory = store.database.parent().expect("a directory");
    let aside = directory.join(".postio-plaintext");
    assert!(aside.join("postio.db").is_file(), "the database is nowhere");
    assert!(
        every_file(&aside.join("blobs"))
            .iter()
            .any(|path| std::fs::read(path).expect("read") == RAW_SOURCE),
        "the raw source is nowhere"
    );
}

/// Every regular file under `root`, at any depth.
fn every_file(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(every_file(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}
