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
