//! `TokenSource`: the credential a session presents, whatever its origin (ADR 0006 Q1).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::secret::{AccountKey, CommandSecretStore, Password, SecretError, SecretStore};
use crate::single_flight::SingleFlight;

/// One instance per account: the IMAP pool and the SMTP path share it, so an invalidation by one is
/// seen by the other (ADR 0006 Q5).
#[async_trait]
pub trait TokenSource: Send + Sync + fmt::Debug {
    /// Best effort: the server is the judge; a caller it overrules should `invalidate` and ask once
    /// more.
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError>;

    async fn invalidate(&self, account: &AccountKey);
}

/// Retries `attempt` once with a fresh credential when `rejected` says the server refused the
/// first. Never loops: a revoked grant would round-trip forever.
/// No retry if the credential did not change. `Err` means no credential exists; `Ok(Err(..))` is
/// the server's answer to the one presented.
pub async fn with_credential<T, E, A, Fut>(
    tokens: &dyn TokenSource,
    key: &AccountKey,
    rejected: impl Fn(&E) -> bool,
    attempt: A,
) -> Result<Result<T, E>, SecretError>
where
    A: Fn(Password) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let credential = tokens.access_token(key).await?;
    let refused = match attempt(credential.clone()).await {
        Ok(value) => return Ok(Ok(value)),
        Err(error) if rejected(&error) => error,
        Err(error) => return Ok(Err(error)),
    };

    tokens.invalidate(key).await;
    let refreshed = tokens.access_token(key).await?;
    if refreshed.expose() == credential.expose() {
        return Ok(Err(refused));
    }
    Ok(attempt(refreshed).await)
}

#[derive(Clone, Debug)]
pub struct StoredPasswordSource {
    store: Arc<dyn SecretStore>,
}

impl StoredPasswordSource {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl TokenSource for StoredPasswordSource {
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
        self.store.retrieve(account).await
    }

    async fn invalidate(&self, _account: &AccountKey) {
        // A stored password does not expire; a rejection is the user's to resolve.
    }
}

/// Caches the broker's token per account so a burst of sessions spawns one process.
#[derive(Debug)]
pub struct BrokerTokenSource {
    command: CommandSecretStore,
    /// `Password` zeroizes on drop, so an evicted token does not linger.
    cache: Mutex<HashMap<AccountKey, Password>>,
    /// One broker run per account at a time: a broker that refreshes as a side effect can rotate
    /// the refresh token (ADR 0006 Q5).
    obtaining: SingleFlight<AccountKey, Result<Password, SecretError>>,
}

impl BrokerTokenSource {
    pub fn new<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            command: CommandSecretStore::new(argv),
            cache: Mutex::new(HashMap::new()),
            obtaining: SingleFlight::default(),
        }
    }

    fn cached(&self, account: &AccountKey) -> Option<Password> {
        self.cache
            .lock()
            .expect("token cache mutex")
            .get(account)
            .cloned()
    }
}

#[async_trait]
impl TokenSource for BrokerTokenSource {
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
        if let Some(token) = self.cached(account) {
            return Ok(token);
        }
        self.obtaining
            .run(account, async {
                // Checked again inside the flight: a caller arriving as the previous run lands
                // takes its answer.
                if let Some(token) = self.cached(account) {
                    return Ok(token);
                }
                let token = self.command.retrieve(account).await?;
                self.cache
                    .lock()
                    .expect("token cache mutex")
                    .insert(account.clone(), token.clone());
                Ok(token)
            })
            .await
    }

    async fn invalidate(&self, account: &AccountKey) {
        self.cache
            .lock()
            .expect("token cache mutex")
            .remove(account);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broker_source_never_reveals_a_token_in_debug() {
        let source = BrokerTokenSource::new(["true"]);
        source.cache.lock().expect("token cache mutex").insert(
            AccountKey::new("ada@example.com"),
            Password::new("t0ps3cret"),
        );

        let rendered = format!("{source:?}");

        assert!(!rendered.contains("t0ps3cret"), "{rendered}");
    }

    /// A broker that refreshes as a side effect can rotate the refresh token (ADR 0006 Q5). Counted
    /// by a file the broker appends to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_callers_run_the_broker_once() {
        let marks = std::env::temp_dir().join(format!("postio-broker-{}", std::process::id()));
        let _ = std::fs::remove_file(&marks);
        let script = format!(
            "printf 'x' >> {}; sleep 0.2; printf 'brokered-token'",
            marks.display()
        );
        let source = Arc::new(BrokerTokenSource::new(["sh", "-c", &script]));
        let account = AccountKey::new("ada@example.com");

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let (source, account) = (Arc::clone(&source), account.clone());
            tasks.spawn(async move { source.access_token(&account).await });
        }
        let mut tokens = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            tokens.push(joined.expect("the task should not panic").expect("a token"));
        }

        let runs = std::fs::read_to_string(&marks).expect("the broker ran at least once");
        let _ = std::fs::remove_file(&marks);

        assert_eq!(runs.len(), 1, "four callers, one process");
        assert_eq!(tokens.len(), 4);
        assert!(
            tokens
                .iter()
                .all(|token| token.expose() == "brokered-token")
        );
    }

    #[derive(Debug)]
    struct Rotating {
        first: String,
        second: String,
        invalidated: std::sync::atomic::AtomicUsize,
        handed_out: std::sync::atomic::AtomicUsize,
    }

    impl Rotating {
        fn new(first: &str, second: &str) -> Self {
            Self {
                first: first.to_owned(),
                second: second.to_owned(),
                invalidated: std::sync::atomic::AtomicUsize::new(0),
                handed_out: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl TokenSource for Rotating {
        async fn access_token(&self, _account: &AccountKey) -> Result<Password, SecretError> {
            use std::sync::atomic::Ordering;
            self.handed_out.fetch_add(1, Ordering::SeqCst);
            Ok(Password::new(
                if self.invalidated.load(Ordering::SeqCst) == 0 {
                    &self.first
                } else {
                    &self.second
                },
            ))
        }

        async fn invalidate(&self, _account: &AccountKey) {
            self.invalidated
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn key() -> AccountKey {
        AccountKey::new("ada@example.com")
    }

    fn presenting(
        seen: Arc<Mutex<Vec<String>>>,
        accepted: &'static str,
    ) -> impl Fn(Password) -> std::future::Ready<Result<&'static str, &'static str>> {
        move |password: Password| {
            seen.lock()
                .expect("seen mutex")
                .push(password.expose().to_owned());
            std::future::ready(if password.expose() == accepted {
                Ok("a session")
            } else {
                Err("rejected")
            })
        }
    }

    #[tokio::test]
    async fn a_rejected_credential_is_invalidated_once_and_retried_once() {
        let source = Rotating::new("stale", "fresh");
        let seen = Arc::new(Mutex::new(Vec::new()));

        let outcome = with_credential(
            &source,
            &key(),
            |_| true,
            presenting(Arc::clone(&seen), "fresh"),
        )
        .await
        .expect("a credential was available");

        assert_eq!(outcome, Ok("a session"));
        assert_eq!(
            *seen.lock().expect("seen mutex"),
            vec!["stale".to_owned(), "fresh".to_owned()],
            "the stale one, then the fresh one, and never a third"
        );
        assert_eq!(
            source.invalidated.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn a_source_with_nothing_new_is_not_retried_against() {
        // `invalidate` is a no-op for stored passwords, so a retry would re-present identical
        // bytes.
        let source = Rotating::new("wrong", "wrong");
        let seen = Arc::new(Mutex::new(Vec::new()));

        let outcome = with_credential(
            &source,
            &key(),
            |_| true,
            presenting(Arc::clone(&seen), "right"),
        )
        .await
        .expect("a credential was available");

        assert_eq!(outcome, Err("rejected"));
        assert_eq!(
            seen.lock().expect("seen mutex").len(),
            1,
            "one round trip, not two"
        );
    }

    #[tokio::test]
    async fn a_failure_that_is_not_a_rejection_spends_no_credential_on_a_retry() {
        // A refused command is the operation's problem; it must not spend a credential.
        let source = Rotating::new("fine", "fresh");
        let seen = Arc::new(Mutex::new(Vec::new()));

        let outcome = with_credential(
            &source,
            &key(),
            |_| false,
            presenting(Arc::clone(&seen), "nothing"),
        )
        .await
        .expect("a credential was available");

        assert_eq!(outcome, Err("rejected"));
        assert_eq!(seen.lock().expect("seen mutex").len(), 1);
        assert_eq!(
            source.invalidated.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "nothing was invalidated"
        );
    }

    #[tokio::test]
    async fn no_credential_at_all_is_a_different_answer_from_a_refused_one() {
        // No credential at all must not arrive looking like a server rejection.
        #[derive(Debug)]
        struct Missing;

        #[async_trait]
        impl TokenSource for Missing {
            async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
                Err(SecretError::NotFound {
                    account: account.account().to_owned(),
                })
            }
            async fn invalidate(&self, _account: &AccountKey) {}
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let outcome = with_credential(
            &Missing,
            &key(),
            |_| true,
            presenting(Arc::clone(&seen), "anything"),
        )
        .await;

        assert!(matches!(outcome, Err(SecretError::NotFound { .. })));
        assert!(
            seen.lock().expect("seen mutex").is_empty(),
            "nothing was presented to any server"
        );
    }
}
