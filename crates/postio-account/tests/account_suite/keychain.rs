#![cfg(target_os = "macos")]

use postio_account::secret::{AccountKey, KeychainSecretStore, Password, SecretError, SecretStore};

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
    // The trait says "stores (or replaces)"; two items per account would make a changed password
    // silently not apply.
    let (store, _scratch) = scratch_keychain();
    let key = AccountKey::new("ada@example.com");

    store.store(&key, &Password::new("first")).await.unwrap();
    store.store(&key, &Password::new("second")).await.unwrap();

    assert_eq!(store.retrieve(&key).await.unwrap().expose(), "second");
}

#[tokio::test]
async fn an_absent_password_is_not_found_rather_than_an_error() {
    // ADR 0014: `store_key` mints a key only on `NotFound`; minting on a transient failure would
    // orphan the existing store.
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
    let (store, _scratch) = scratch_keychain();
    assert_eq!(store.describe(), "keychain");
}

#[test]
fn the_platform_keyring_is_the_keychain_here() {
    // `SecretSource::Keyring` must resolve to the Keychain on macOS, not the Secret Service.
    assert_eq!(
        postio_account::secret::platform_keyring().describe(),
        "keychain"
    );
}

#[tokio::test]
async fn the_unlock_hint_names_software_a_mac_user_has() {
    // A locked-keychain message must not point at "Passwords and Keys", which does not exist on
    // macOS.
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
