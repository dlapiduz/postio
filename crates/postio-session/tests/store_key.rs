//! The store key, from the keyring to the store — ADR 0014 Q3.
//!
//! The material itself is `postio-storage`'s and tested there. What this is
//! about is the *service*: minting one on first run, reading the same one
//! back on every run after, and what happens when the keyring will not answer.
//!
//! Nothing here touches a real keyring. `MemorySecretStore::reopen` is a
//! restart with the same keyring, which is exactly the question — a service
//! that minted a fresh key on the second open would look fine on first run
//! and lose every mailbox on the second.

use std::sync::Arc;

use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretError, SecretStore};
use postio_session::{STORE_KEY_ENTRY, store_key};

fn entry() -> AccountKey {
    AccountKey::new(STORE_KEY_ENTRY)
}

#[tokio::test]
async fn a_first_run_mints_a_key_and_keeps_it() {
    let keyring = MemorySecretStore::new();
    assert!(keyring.is_empty(), "nothing has ever been stored");

    let key = store_key(&keyring).await.expect("a first run mints one");

    let kept = keyring
        .retrieve(&entry())
        .await
        .expect("and writes it down");
    assert_eq!(
        kept.expose(),
        key.to_hex().as_str(),
        "the key handed back is the key that was kept — anything else and the \
         store is encrypted under something nobody can read again"
    );
}

#[tokio::test]
async fn a_second_open_reads_the_key_back_rather_than_minting_another() {
    // The whole point. A store is encrypted under the key of its first open,
    // and a service that minted a new one per run would produce a mail client
    // that loses its mailbox every time it starts.
    let keyring = MemorySecretStore::new();

    let first = store_key(&keyring).await.expect("first open");
    let second = store_key(&keyring.reopen()).await.expect("second open");

    assert_eq!(first.to_hex(), second.to_hex());
    assert_eq!(keyring.len(), 1, "and one entry, not two");
}

#[tokio::test]
async fn a_locked_keyring_refuses_and_says_how_to_unlock() {
    // No plaintext fallback, and no fresh key either. Minting one here would
    // be worse than refusing: it would encrypt the next thing written under a
    // key the existing store knows nothing about, and the mailbox would be
    // gone rather than merely unavailable.
    let keyring = MemorySecretStore::locked();

    let refused = store_key(&keyring)
        .await
        .expect_err("a locked keyring cannot answer");

    assert!(
        matches!(refused, SecretError::Locked { .. }),
        "the variant has to survive, because it is what routes to the unlock \
         surface rather than to onboarding: {refused:?}"
    );
    let said = refused.to_string();
    assert!(
        said.contains("locked") && said.contains("unlock"),
        "and it has to tell the user what to do: {said}"
    );
    assert!(
        keyring.is_empty(),
        "nothing was written to a locked keyring"
    );
}

#[tokio::test]
async fn an_entry_that_is_not_a_key_is_refused_rather_than_replaced() {
    // A corrupt entry is a store that cannot be opened; a *replaced* entry is
    // a store that can never be opened again. So this refuses, loudly, and
    // leaves what is there for a person to deal with.
    let keyring = MemorySecretStore::new();
    keyring
        .store(&entry(), &Password::new("this is not a key"))
        .await
        .expect("the fixture writes");

    let refused = store_key(&keyring).await.expect_err("that will not parse");

    assert!(
        !matches!(refused, SecretError::NotFound { .. }),
        "a corrupt entry is not a missing one: {refused:?}"
    );
    assert_eq!(
        keyring
            .retrieve(&entry())
            .await
            .expect("still there")
            .expose(),
        "this is not a key",
        "the entry that could not be parsed must still be there afterwards"
    );
}

#[tokio::test]
async fn an_empty_entry_is_a_failed_write_and_is_minted_over() {
    // The one entry that is safe to replace: nothing can have been encrypted
    // under an empty key, so the store behind it is either absent or already
    // unopenable. Treating it as a first run is what gives a half-finished
    // first run a way out, and it is the same tolerance `startup_route`
    // already extends to an empty password.
    let keyring = MemorySecretStore::new();
    keyring
        .store(&entry(), &Password::new(""))
        .await
        .expect("the fixture writes");

    let key = store_key(&keyring).await.expect("a fresh key");

    assert_eq!(
        keyring.retrieve(&entry()).await.expect("kept").expose(),
        key.to_hex().as_str()
    );
}

// ---------------------------------------------------------------------------
// Before there is a runtime, because there is no store yet either
// ---------------------------------------------------------------------------

/// Deliberately not a `#[tokio::test]`: the whole point of the blocking form
/// is that startup has no runtime when it needs the key. The store has to be
/// open before the command bus can be built, and the bus has to exist before
/// the runtime that pumps it.
#[test]
fn the_key_can_be_read_with_no_runtime_running() {
    let keyring = MemorySecretStore::new();

    let first = postio_session::store_key_blocking(&keyring).expect("a first run");
    let second = postio_session::store_key_blocking(&keyring.reopen()).expect("a second run");

    assert_eq!(first.to_hex(), second.to_hex());
}

#[test]
fn a_locked_keyring_means_there_is_no_store_to_open() {
    // What `postio_app::run` branches on. `open_store` is only reached on the
    // `Ok` arm, so this is the point at which "a locked keyring means the mail
    // does not open" is decided -- before a `Database` exists, rather than by
    // something downstream noticing later.
    let refused = postio_session::store_key_blocking(&MemorySecretStore::locked())
        .expect_err("a locked keyring cannot answer");

    assert!(matches!(refused, SecretError::Locked { .. }));
    assert!(
        refused.to_string().contains("unlock"),
        "and the sentence that reaches the user says what to do: {refused}"
    );
}

// ---------------------------------------------------------------------------
// Nothing about the key reaches a log
// ---------------------------------------------------------------------------

/// A writer every `tracing` line lands in, so a test can read them back.
///
/// The same shape `postio-runtime`'s `logging_privacy.rs` uses, for the same
/// reason: a rule nothing checks is a rule that lasts until the next person
/// adds a `?key` to a `tracing` call because it would have been convenient
/// that once.
#[derive(Clone, Default)]
struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn no_key_material_reaches_the_log_at_any_level() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();

    let minted = {
        let _guard = tracing::subscriber::set_default(subscriber);
        // Every path through the service, in one capture: a first run, a
        // reopen, a locked keyring and a corrupt entry.
        let keyring = MemorySecretStore::new();
        let minted = store_key(&keyring).await.expect("first run");
        store_key(&keyring.reopen()).await.expect("second run");
        let _ = store_key(&MemorySecretStore::locked()).await;

        let corrupt = MemorySecretStore::new();
        corrupt
            .store(&entry(), &Password::new("not-a-key"))
            .await
            .expect("the fixture writes");
        let _ = store_key(&corrupt).await;
        minted
    };

    let log = captured.text();
    let hex = minted.to_hex();
    assert!(!log.contains(hex.as_str()), "the key is in the log: {log}");
    // And no fragment of it either: a truncated key is still most of a key,
    // and half of thirty-two bytes is not a brute force anybody would mind.
    for window in hex.as_bytes().windows(16) {
        let fragment = std::str::from_utf8(window).expect("hex is ascii");
        assert!(
            !log.contains(fragment),
            "a fragment of the key is in the log ({fragment}): {log}"
        );
    }
}

// ---------------------------------------------------------------------------
// The pre-release migration, at the seam that runs it
// ---------------------------------------------------------------------------

/// A plaintext store: an unencrypted database with one message pointing at a
/// bare-bytes blob, which is what a checkout from before ADR 0014 has on disk.
///
/// Written out here rather than reached for, because the point of these two
/// tests is the *wiring*: `postio_storage::encrypt` is proven in its own crate,
/// and what nothing there can show is whether `open_store_at` calls it. That is
/// this project's characteristic bug — a layer that is built, tested, and never
/// mounted — so the test has to come in through the door the app comes in
/// through.
fn a_plaintext_store(directory: &std::path::Path) -> (std::path::PathBuf, String) {
    use postio_model::Message;
    use postio_storage::repository::MessageRepository;

    let path = directory.join("postio.db");
    let mut connection = rusqlite::Connection::open(&path).expect("a plaintext database");
    postio_storage::migrate(&mut connection).expect("migrate");

    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    message.subject = Some("Zarquon".to_owned());
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");

    let raw = b"From: ada@example.com\r\nSubject: Zarquon\r\n\r\nThursday.\r\n";
    let digest = blake3::hash(raw).to_hex().to_string();
    let shard = directory
        .join("blobs")
        .join(&digest[0..2])
        .join(&digest[2..4]);
    std::fs::create_dir_all(&shard).expect("shard directories");
    std::fs::write(shard.join(&digest[4..]), raw).expect("write the blob");

    connection
        .execute(
            "UPDATE messages SET raw_blob_id = ?1 WHERE id = ?2",
            rusqlite::params![digest, id.get()],
        )
        .expect("point the message at its source");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .expect("checkpoint");
    drop(connection);
    (path, digest)
}

#[test]
fn opening_a_plaintext_store_encrypts_it_first() {
    let directory = tempfile::tempdir().expect("a directory");
    let (path, old_id) = a_plaintext_store(directory.path());
    let key = postio_storage::key::StoreKey::from_bytes([0x2a; 32]);

    let (database, blobs) =
        postio_session::open_store_at(&path, &key).expect("the store opens after migrating");

    let connection = database.connection().expect("checkout");
    let (subject, raw): (String, String) = connection
        .query_row("SELECT subject, raw_blob_id FROM messages", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("the message survived");
    assert_eq!(subject, "Zarquon");
    assert_ne!(raw, old_id, "the row still carries the unkeyed digest");
    assert_eq!(
        blobs
            .get(&postio_model::BlobId::new(raw))
            .expect("the raw source reads through the encrypted store"),
        b"From: ada@example.com\r\nSubject: Zarquon\r\n\r\nThursday.\r\n"
    );

    // And it is a store the *next* open can read, which is the only version of
    // this that matters.
    drop(connection);
    drop(database);
    postio_session::open_store_at(&path, &key).expect("and it opens again");
}

#[test]
fn a_store_with_work_still_queued_refuses_and_says_what_to_do() {
    let directory = tempfile::tempdir().expect("a directory");
    let (path, _) = a_plaintext_store(directory.path());
    let connection = rusqlite::Connection::open(&path).expect("open");
    connection
        .execute(
            "INSERT INTO operation_queue (account_id, op_type, created_at, updated_at)
             VALUES ((SELECT id FROM accounts LIMIT 1), 'flag', 0, 0)",
            [],
        )
        .expect("enqueue");
    drop(connection);

    let key = postio_storage::key::StoreKey::from_bytes([0x2b; 32]);
    let said = postio_session::open_store_at(&path, &key)
        .expect_err("a store with undrained work must not be migrated");

    // The sentence goes on a screen (#404), so it has to name the problem and
    // the way out rather than a state a person cannot act on.
    assert!(
        said.contains("server") && said.to_lowercase().contains("syncing"),
        "the message must say what to do next: {said}"
    );
}
