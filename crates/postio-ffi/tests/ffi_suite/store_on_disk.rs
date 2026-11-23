use std::sync::Arc;

use async_trait::async_trait;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretError, SecretStore};
use postio_ffi::{Session, SessionError, SessionOptions};

#[derive(Debug)]
struct LockedKeyring;

#[async_trait]
impl SecretStore for LockedKeyring {
    fn describe(&self) -> &'static str {
        "locked-for-test"
    }

    async fn store(&self, _key: &AccountKey, _password: &Password) -> Result<(), SecretError> {
        Err(self.locked())
    }

    async fn retrieve(&self, _key: &AccountKey) -> Result<Password, SecretError> {
        Err(self.locked())
    }

    async fn delete(&self, _key: &AccountKey) -> Result<(), SecretError> {
        Err(self.locked())
    }
}

impl LockedKeyring {
    fn locked(&self) -> SecretError {
        SecretError::Locked {
            keyring: "login".to_string(),
            account: "postio-store-key".to_string(),
        }
    }
}

#[test]
fn a_session_opens_over_a_store_on_disk() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");

    let session =
        Session::open(SessionOptions::at(&path).with_secrets(Arc::new(MemorySecretStore::new())))
            .expect("a session over a store on disk");

    assert!(
        session.is_open(),
        "the session reported open but holds no store"
    );
    assert!(
        path.exists(),
        "no database at {} -- the session opened nothing",
        path.display()
    );
    session.shutdown();
}

/// Counts key writes: the store opens either way, so reuse of the key can be shown only by the
/// mints.
#[derive(Debug)]
struct CountingKeyring {
    inner: MemorySecretStore,
    mints: std::sync::atomic::AtomicUsize,
}

impl CountingKeyring {
    fn new() -> Self {
        Self {
            inner: MemorySecretStore::new(),
            mints: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn mints(&self) -> usize {
        self.mints.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl SecretStore for CountingKeyring {
    fn describe(&self) -> &'static str {
        "counting-for-test"
    }

    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError> {
        self.mints.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.store(key, password).await
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        self.inner.retrieve(key).await
    }

    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        self.inner.delete(key).await
    }
}

#[test]
fn the_store_survives_a_second_session_over_the_same_path() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");
    let counting = Arc::new(CountingKeyring::new());
    let secrets: Arc<dyn SecretStore> = counting.clone();

    let first = Session::open(SessionOptions::at(&path).with_secrets(secrets.clone()))
        .expect("a first session");
    first.shutdown();
    drop(first);
    assert_eq!(
        counting.mints(),
        1,
        "the first run should mint exactly one key"
    );

    let second = Session::open(SessionOptions::at(&path).with_secrets(secrets))
        .expect("a second session over the same store");
    assert!(second.is_open());
    second.shutdown();

    // A second mint means the store was re-keyed every launch: harmless until `PRAGMA key` (#300),
    // then every message is unreadable.
    assert_eq!(
        counting.mints(),
        1,
        "the second session minted a new key instead of reading the first one's"
    );
}

#[test]
fn a_locked_keyring_is_its_own_error_and_not_a_broken_store() {
    // `KeyringLocked` routes to "unlock and retry", `StoreUnavailable` to "your store is broken";
    // swapped, someone with working mail lands in onboarding (ADR 0014).
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");

    // Matched rather than `expect_err`, which would need `Debug` on `Session`.
    let error = match Session::open(SessionOptions::at(&path).with_secrets(Arc::new(LockedKeyring)))
    {
        Ok(_) => panic!("a locked keyring must not yield a session"),
        Err(error) => error,
    };

    match error {
        SessionError::KeyringLocked { message } => {
            assert!(
                !message.is_empty(),
                "the case is right but the message is empty, so the surface has nothing to show"
            );
        }
        other => panic!("expected KeyringLocked, got {other:?}"),
    }
}

#[test]
fn a_locked_keyring_opens_no_store_at_all() {
    // No plaintext fallback (ADR 0014): not a degraded session, and no database file left behind.
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");

    let _ = Session::open(SessionOptions::at(&path).with_secrets(Arc::new(LockedKeyring)));

    assert!(
        !path.exists(),
        "a database was created despite the keyring being locked, so something \
         opened a store it had no key for"
    );
}

#[test]
fn a_store_from_another_build_says_start_over_not_try_again() {
    // `StoreUnavailable` means "try again"; a store whose schema no migration reaches is refused
    // every time, so the Mac must not offer it.
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let key = postio_session::store_key_blocking(secrets.as_ref()).expect("a store key");
    postio_session::blocking::now(postio_storage::Store::create_at_schema(
        &path,
        &key.derive(postio_storage::key::Purpose::Database),
        "CREATE TABLE written_by_a_stranger (id INTEGER PRIMARY KEY);",
        "",
    ))
    .expect("a store at a schema no step leads from");

    let error = match Session::open(SessionOptions::at(&path).with_secrets(secrets)) {
        Ok(_) => panic!("a store from another build must not yield a session"),
        Err(error) => error,
    };

    match error {
        SessionError::StoreFromAnotherBuild { message } => {
            assert!(!message.is_empty(), "the surface has nothing to show");
        }
        other => panic!("expected StoreFromAnotherBuild, got {other:?}"),
    }
}

#[test]
fn starting_over_a_store_from_another_build_leaves_one_that_opens() {
    // Start over sets the store aside under its own key, not deleted, and a fresh one takes its
    // place.
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());
    let key = postio_session::store_key_blocking(secrets.as_ref()).expect("a store key");
    postio_session::blocking::now(postio_storage::Store::create_at_schema(
        &path,
        &key.derive(postio_storage::key::Purpose::Database),
        "CREATE TABLE written_by_a_stranger (id INTEGER PRIMARY KEY);",
        "",
    ))
    .expect("a store at a schema no step leads from");

    let started =
        postio_ffi::start_over_with(SessionOptions::at(&path).with_secrets(secrets.clone()))
            .unwrap_or_else(|error| panic!("the store was not started over: {error}"));
    assert!(
        std::path::Path::new(&started.set_aside).is_dir(),
        "the old store was not set aside where it said: {}",
        started.set_aside
    );
    assert_eq!(started.accounts, 0, "the stranger's store held no accounts");

    let session = Session::open(SessionOptions::at(&path).with_secrets(secrets))
        .unwrap_or_else(|error| panic!("the fresh store did not open: {error}"));
    assert!(session.is_open());
    session.shutdown();
}

#[test]
fn a_store_under_another_key_says_start_over_and_starting_over_opens() {
    // The key does not change between attempts, so "Try again" is a dead end; nothing in the old
    // store decrypts, so no account comes across (#1598).
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");
    let stranger = postio_storage::key::StoreKey::from_bytes([0x2a; 32]);
    drop(
        postio_session::blocking::now(postio_session::open_store_at(&path, &stranger))
            .expect("a store under another installation's key"),
    );
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());

    match Session::open(SessionOptions::at(&path).with_secrets(secrets.clone())) {
        Ok(_) => panic!("a store under another key must not yield a session"),
        Err(SessionError::StoreUnderAnotherKey { message }) => {
            assert!(!message.is_empty(), "the surface has nothing to show");
        }
        Err(other) => panic!("expected StoreUnderAnotherKey, got {other:?}"),
    }

    let started =
        postio_ffi::start_over_with(SessionOptions::at(&path).with_secrets(secrets.clone()))
            .unwrap_or_else(|error| panic!("the store was not started over: {error}"));
    assert!(
        std::path::Path::new(&started.set_aside)
            .join("postio.db")
            .exists(),
        "the old store was not set aside where it said: {}",
        started.set_aside
    );
    let session = Session::open(SessionOptions::at(&path).with_secrets(secrets))
        .unwrap_or_else(|error| panic!("the fresh store did not open: {error}"));
    assert!(session.is_open());
    session.shutdown();
}

#[test]
fn the_refusal_is_worded_once_for_both_apps() {
    // The words are `postio_ui::focus_state`'s, read here rather than copied to drift.
    use postio_ui::focus_state as words;
    let said = postio_ffi::store_refusal_words();
    assert_eq!(said.cant_open, words::CANT_OPEN_MAIL);
    assert_eq!(said.try_again, words::TRY_AGAIN);
    assert_eq!(said.from_another_build, words::STORE_FROM_ANOTHER_VERSION);
    assert_eq!(said.start_over_sentence, words::START_OVER);
    assert_eq!(said.under_another_key, words::STORE_UNDER_ANOTHER_KEY);
    assert_eq!(
        said.start_over_under_another_key_sentence,
        words::START_OVER_UNDER_ANOTHER_KEY
    );
    assert_eq!(said.start_over, words::START_A_FRESH_STORE);
    assert_eq!(said.starting_over, words::STARTING_A_FRESH_STORE);
    assert_eq!(
        postio_ffi::started_over_words("/stores/set-aside/when".to_owned()),
        "Started a fresh store. The old one is in /stores/set-aside/when"
    );
}
