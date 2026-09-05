//! Opening a session over a real store, and what a locked keyring means.
//!
//! ADR 0014 put the store's master key in the OS keyring, so "open the store"
//! is really "ask the keyring, then open the store" — and the two ways that
//! can fail are not the same failure. A locked keyring is recoverable by the
//! user and its remedy is a different surface from a store that will not open.
//! These tests exist to keep those two apart at the boundary, where a frontend
//! has nothing but the error case to route on.
//!
//! Nothing here touches the user's real keyring: every case injects its own
//! secret store. A test that reached for the login keyring would prompt on a
//! developer's machine and hang on a headless one.

use std::sync::Arc;

use async_trait::async_trait;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretError, SecretStore};
use postio_ffi::{Session, SessionError, SessionOptions};

/// A keyring that is present and refuses, which is the case ADR 0014 cares
/// about: not "there is no key" but "there is a key and you cannot have it".
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
    // The store is on disk and not merely claimed: a constructor that
    // returned Ok while opening nothing would satisfy every other assertion
    // in this file.
    assert!(
        path.exists(),
        "no database at {} -- the session opened nothing",
        path.display()
    );
    session.shutdown();
}

/// A real secret store with a tally of how often a key was *written*.
///
/// Needed because ADR 0014 lands in stages and the store is not encrypted
/// yet (`open_store` still takes the key without using it, pending #300/#301).
/// So "the second session reused the first one's key" cannot be shown by the
/// store opening — it would open either way. Counting the mints shows it.
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
    // The key is minted on first run and read back on the next. Reusing one
    // secret store across both is what makes this the real sequence rather
    // than two independent first runs.
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

    // The assertion that matters. A second mint would mean the store was
    // re-keyed on every launch -- harmless today, because nothing is
    // encrypted under it yet, and catastrophic the moment #300 issues
    // `PRAGMA key`: every previous message would become unreadable. Better to
    // hold the line now, while the failure is still cheap.
    assert_eq!(
        counting.mints(),
        1,
        "the second session minted a new key instead of reading the first one's"
    );
}

#[test]
fn a_locked_keyring_is_its_own_error_and_not_a_broken_store() {
    // The routing this protects: `KeyringLocked` sends the user to "unlock
    // and retry", `StoreUnavailable` to "your store is broken". Getting them
    // the wrong way round sends somebody with working mail to onboarding,
    // which is the failure ADR 0014 names explicitly.
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");

    // Matched rather than `expect_err`, which would need `Debug` on `Session`
    // -- and a session that carries a live store, a runtime handle and a
    // command bus has no business having a derived one.
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
    // ADR 0014's "no plaintext fallback", asserted where it can be seen: not
    // a degraded session, not an empty one, no database file left behind.
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio.db");

    let _ = Session::open(SessionOptions::at(&path).with_secrets(Arc::new(LockedKeyring)));

    assert!(
        !path.exists(),
        "a database was created despite the keyring being locked, so something \
         opened a store it had no key for"
    );
}
