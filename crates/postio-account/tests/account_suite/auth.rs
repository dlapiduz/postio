use std::sync::Arc;

use postio_account::auth::{BrokerTokenSource, StoredPasswordSource, TokenSource};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretError, SecretStore};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("postio-auth-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn counting_broker(directory: &std::path::Path) -> BrokerTokenSource {
    let counter = directory.join("count");
    let script = format!(
        "n=$(cat {c} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {c}; echo token-$n",
        c = counter.display()
    );
    BrokerTokenSource::new(["sh".to_string(), "-c".to_string(), script])
}

fn key() -> AccountKey {
    AccountKey::new("ada@example.com")
}

#[tokio::test]
async fn a_broker_token_is_cached_between_calls() {
    let dir = scratch("cached");
    let source = counting_broker(&dir);

    let first = source.access_token(&key()).await.expect("a token");
    let second = source.access_token(&key()).await.expect("a token");

    assert_eq!(first.expose(), "token-1");
    assert_eq!(
        second.expose(),
        "token-1",
        "a second ask within one lifetime must not re-run the broker"
    );
}

#[tokio::test]
async fn invalidate_reobtains_on_the_next_ask() {
    let dir = scratch("invalidate");
    let source = counting_broker(&dir);

    let before = source.access_token(&key()).await.expect("a token");
    source.invalidate(&key()).await;
    let after = source.access_token(&key()).await.expect("a token");

    assert_eq!(before.expose(), "token-1");
    assert_eq!(
        after.expose(),
        "token-2",
        "a rejected token must be re-obtained, not served from the cache"
    );
}

#[tokio::test]
async fn the_cache_is_per_account() {
    let dir = scratch("per-account");
    let source = counting_broker(&dir);

    let ada = source.access_token(&key()).await.expect("a token");
    let lena = source
        .access_token(&AccountKey::new("lena@example.com"))
        .await
        .expect("a token");
    source
        .invalidate(&AccountKey::new("lena@example.com"))
        .await;
    let ada_again = source.access_token(&key()).await.expect("a token");

    assert_eq!(ada.expose(), "token-1");
    assert_eq!(lena.expose(), "token-2");
    assert_eq!(
        ada_again.expose(),
        "token-1",
        "invalidating one account must not evict another's token"
    );
}

#[tokio::test]
async fn a_failing_broker_is_a_command_error_not_a_hang() {
    let source = BrokerTokenSource::new(["false".to_string()]);

    let result = source.access_token(&key()).await;

    assert!(
        matches!(result, Err(SecretError::Command { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_stored_password_answers_both_trait_methods() {
    let store = MemorySecretStore::new();
    store
        .store(&key(), &Password::new("hunter2"))
        .await
        .expect("the store accepts a password");
    let source = StoredPasswordSource::new(Arc::new(store));

    let token = source.access_token(&key()).await.expect("the password");
    source.invalidate(&key()).await;
    let again = source.access_token(&key()).await.expect("still there");

    assert_eq!(token.expose(), "hunter2");
    assert_eq!(again.expose(), "hunter2");
}

#[tokio::test]
async fn a_missing_stored_password_says_not_found() {
    let source = StoredPasswordSource::new(Arc::new(MemorySecretStore::new()));

    let result = source.access_token(&key()).await;

    assert!(
        matches!(result, Err(SecretError::NotFound { .. })),
        "{result:?}"
    );
}
