//! The macOS Keychain store, against a keychain the test owns.
//!
//! Never the login keychain: a test that reached for it would prompt on a
//! developer's machine and hang every headless run. Each case creates a
//! keychain file in a temp directory, uses it, and throws it away.

#![cfg(target_os = "macos")]

use postio_imap::secret::{AccountKey, KeychainSecretStore, Password, SecretError, SecretStore};

/// A store over a keychain that exists only for this test.
fn scratch_keychain() -> (KeychainSecretStore, tempfile::TempDir) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let path = scratch.path().join("postio-test.keychain");
    let store = KeychainSecretStore::at(&path, "test-password").expect("a keychain the test owns");
    (store, scratch)
}

#[tokio::test]
async fn a_password_round_trips() {
    let (store, _scratch) = scratch_keychain();
    let key = AccountKey::new("ada@example.com");

    store
        .store(&key, &Password::new("hunter2"))
        .await
        .expect("the password is stored");

    let read = store.retrieve(&key).await.expect("the password comes back");
    assert_eq!(read.expose(), "hunter2");
}

#[tokio::test]
async fn storing_twice_replaces_rather_than_duplicates() {
    // The trait says "stores (or replaces)". A keychain that accumulated two
    // items for one account would answer with whichever it found first, which
    // is a password that silently stops changing when the user updates it.
    let (store, _scratch) = scratch_keychain();
    let key = AccountKey::new("ada@example.com");

    store.store(&key, &Password::new("first")).await.unwrap();
    store.store(&key, &Password::new("second")).await.unwrap();

    assert_eq!(store.retrieve(&key).await.unwrap().expose(), "second");
}

#[tokio::test]
async fn an_absent_password_is_not_found_rather_than_an_error() {
    // The distinction ADR 0014 turns on. `store_key` treats `NotFound` as a
    // first run and mints a key; anything else it refuses — because minting
    // on a transient failure would encrypt the next write under a key the
    // existing store knows nothing about, and the mailbox would be gone
    // rather than merely unavailable.
    let (store, _scratch) = scratch_keychain();
    let key = AccountKey::new("nobody@example.com");

    match store.retrieve(&key).await {
        Err(SecretError::NotFound { account }) => {
            assert_eq!(account, "nobody@example.com");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn deleting_an_absent_password_succeeds() {
    // The trait requires it: "Removing an absent password succeeds."
    let (store, _scratch) = scratch_keychain();
    let key = AccountKey::new("nobody@example.com");
    store.delete(&key).await.expect("deleting nothing is fine");
}

#[tokio::test]
async fn a_deleted_password_is_gone() {
    let (store, _scratch) = scratch_keychain();
    let key = AccountKey::new("ada@example.com");

    store.store(&key, &Password::new("hunter2")).await.unwrap();
    store.delete(&key).await.unwrap();

    assert!(
        matches!(
            store.retrieve(&key).await,
            Err(SecretError::NotFound { .. })
        ),
        "the password survived its own deletion"
    );
}

#[tokio::test]
async fn two_accounts_do_not_see_each_other() {
    let (store, _scratch) = scratch_keychain();
    let ada = AccountKey::new("ada@example.com");
    let grace = AccountKey::new("grace@example.com");

    store
        .store(&ada, &Password::new("ada-secret"))
        .await
        .unwrap();
    store
        .store(&grace, &Password::new("grace-secret"))
        .await
        .unwrap();

    assert_eq!(store.retrieve(&ada).await.unwrap().expose(), "ada-secret");
    assert_eq!(
        store.retrieve(&grace).await.unwrap().expose(),
        "grace-secret"
    );
}

#[test]
fn the_store_says_which_backend_it_is() {
    // `describe()` round-trips through `config.toml`, so it has to name this
    // backend distinctly from the Secret Service one.
    let (store, _scratch) = scratch_keychain();
    assert_eq!(store.describe(), "keychain");
}

#[test]
fn the_platform_keyring_is_the_keychain_here() {
    // `SecretSource::Keyring` in `config.toml` means "wherever this system
    // keeps secrets". On macOS that is the Keychain, and a build that still
    // reached for the Secret Service would fail at the first D-Bus call —
    // which is exactly how this was found.
    assert_eq!(
        postio_imap::secret::platform_keyring().describe(),
        "keychain"
    );
}

#[tokio::test]
async fn the_unlock_hint_names_software_a_mac_user_has() {
    // The message a locked keychain produces is the only instruction the user
    // gets, and pointing them at "Passwords and Keys" — which does not exist
    // on macOS — is worse than saying nothing.
    let error = SecretError::Locked {
        keyring: "login".to_string(),
        account: "ada@example.com".to_string(),
    };
    let text = error.to_string();
    assert!(
        text.contains("Keychain Access"),
        "the hint does not name the application to open: {text}"
    );
    assert!(
        !text.contains("Passwords and Keys"),
        "the hint still names GNOME's application: {text}"
    );
}
