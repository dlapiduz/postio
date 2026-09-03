//! Obtaining the credential a session presents: a strategy, not a build.
//!
//! ADR 0006 Q1. An account authenticates with *something* — an app-specific
//! password from the keyring today, an OAuth access token tomorrow — and the
//! session layer should not care which. [`TokenSource`] is that seam: ask it
//! for a currently-valid credential, and tell it when the server rejected the
//! one it gave you.
//!
//! Two sources ship first, both networkless:
//!
//! * [`StoredPasswordSource`] — the password world, over any
//!   [`SecretStore`]. `invalidate` is a no-op: a stored password does not
//!   expire, and a rejected one is the user's to change.
//! * [`BrokerTokenSource`] — the delegation world: run a program (`oama`,
//!   `ortie`, `mutt_oauth2.py`) and treat its first line of output as the
//!   token. The broker owns refresh, storage and the provider relationship;
//!   Postio owns exactly one obligation on top of
//!   [`CommandSecretStore`] — **expiry
//!   semantics**. The token is cached per account so a connection pool does
//!   not spawn a process per session, and [`TokenSource::invalidate`] drops
//!   the cache so the next ask re-runs the broker. Without that, delegation
//!   works for exactly one token lifetime and then looks like a broken
//!   account.
//!
//! What deliberately does not live here: the OAuth authorization flow itself
//! (#192), and the routing that turns a rejected credential into something
//! the user is asked to fix — that belongs at the layer which saw the server
//! say no. This module is the seam they plug into.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::secret::{AccountKey, CommandSecretStore, Password, SecretError, SecretStore};
use crate::single_flight::SingleFlight;

/// Where a session's credential comes from.
///
/// One instance per account is the intended shape (ADR 0006 Q5): the
/// account's IMAP pool and its SMTP path share it, so an invalidation by one
/// is seen by the other.
#[async_trait]
pub trait TokenSource: Send + Sync + fmt::Debug {
    /// A currently-valid credential for `account`.
    ///
    /// "Valid" is best effort: the server is the judge, and a caller it
    /// overrules should [`invalidate`](Self::invalidate) and ask once more
    /// before giving up.
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError>;

    /// The server rejected the credential this source last handed out.
    ///
    /// The next [`access_token`](Self::access_token) must re-obtain rather
    /// than repeat itself. Invalidating an account nothing is cached for is
    /// fine and does nothing.
    async fn invalidate(&self, account: &AccountKey);
}

// ---------------------------------------------------------------------------
// Presenting one, and what to do when it is refused
// ---------------------------------------------------------------------------

/// Runs `attempt` with the account's credential, and gives it exactly one
/// more go with a fresh one if the server refused the first.
///
/// # Why this is a function rather than a paragraph in two places
///
/// ADR 0006 Q5 asks for the same discipline wherever a credential meets a
/// server, and Postio has two such places: the IMAP pool opening a
/// connection, and the SMTP path opening a send. Written twice, they drift —
/// and the way they drift is that one of them keeps retrying, which against a
/// revoked grant is an endless pair of round trips to a server that has
/// already said no.
///
/// Three rules, and all three matter:
///
/// * **One retry.** Not a loop: the source has had its chance to produce
///   something better, and a second failure is the user's to resolve.
/// * **Only on a rejection.** `rejected` decides. A refused `MOVE` is the
///   operation's problem and must not spend a credential on it.
/// * **Not at all if the credential did not change.** Re-presenting identical
///   bytes to a server that just refused them is a wasted round trip, and it
///   is the common case: `invalidate` on a stored password is a documented
///   no-op.
///
/// The two failures stay apart in the return type. `Err` is "there is no
/// credential to present" — the keyring is locked, the grant is gone — and
/// `Ok(Err(..))` is what the server said about the one that was presented.
/// Callers route those to different places, so flattening them here would
/// only mean unflattening them twice.
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

// ---------------------------------------------------------------------------
// Stored passwords
// ---------------------------------------------------------------------------

/// The password world behind the [`TokenSource`] seam.
///
/// Reads whatever [`SecretStore`] it is given — the keyring in the
/// application, [`MemorySecretStore`](crate::secret::MemorySecretStore) in a
/// test — so a caller holds one type whether the account authenticates with
/// a password or a token.
#[derive(Clone, Debug)]
pub struct StoredPasswordSource {
    store: Arc<dyn SecretStore>,
}

impl StoredPasswordSource {
    /// A source reading `store`.
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
        // A stored password does not expire. A server rejecting it is an
        // `Auth` condition for the user to resolve, never a refetch — that
        // routing is the session layer's job, not this type's.
    }
}

// ---------------------------------------------------------------------------
// Broker
// ---------------------------------------------------------------------------

/// Runs a broker program and caches what it prints, per account.
///
/// The cache is what makes a pool affordable — several sessions opening in a
/// burst cost one process, not one each — and [`invalidate`] is what keeps
/// the cache honest once a server disagrees with it.
///
/// [`invalidate`]: TokenSource::invalidate
#[derive(Debug)]
pub struct BrokerTokenSource {
    command: CommandSecretStore,
    /// Tokens already obtained. `Password` zeroizes on drop, so an evicted
    /// or replaced token does not linger.
    cache: Mutex<HashMap<AccountKey, Password>>,
    /// One broker run per account at a time, its result shared.
    ///
    /// A pool opening three sessions on a cold cache used to spawn three
    /// processes. Brokers are idempotent reads, so that was harmless and
    /// wasteful — until a broker that refreshes as a side effect, which is
    /// what `oama` and `mutt_oauth2.py` do, meets a provider that rotates its
    /// refresh token (ADR 0006 Q5).
    obtaining: SingleFlight<AccountKey, Result<Password, SecretError>>,
}

impl BrokerTokenSource {
    /// A source that runs `argv` — program and arguments, e.g.
    /// `["oama", "access", "ada@example.com"]`.
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

    /// The token already obtained for `account`, if there is one.
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
                // Checked again inside the flight, for the reason
                // `OwnClientTokenSource` gives: a caller that arrives as the
                // previous run lands should take its answer rather than spawn
                // a process to learn the same thing.
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

    /// A pool opening three sessions on a cold cache used to spawn three
    /// broker processes. Harmless while a broker is a pure read, and not
    /// harmless once it refreshes as a side effect — which is what `oama` and
    /// `mutt_oauth2.py` do — against a provider that rotates its refresh
    /// token on every use (ADR 0006 Q5).
    ///
    /// Counted by having the broker leave a mark: the file it appends to is
    /// the only honest record of how many processes actually ran.
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

    // -----------------------------------------------------------------------
    // Presenting a credential, and the one retry
    // -----------------------------------------------------------------------

    /// Hands out `first` until invalidated, then `second`, counting both.
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

    /// Records every credential presented, so a test asserts what reached the
    /// server rather than only what came back.
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
        // Every stored password: `invalidate` is a documented no-op, so a
        // retry would re-present identical bytes to a server that has just
        // refused them.
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
        // A refused command is the operation's problem, not the
        // connection's, and asking the source for a fresh token over one
        // would be a refresh nobody needed.
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
        // The keyring is locked, or the grant is gone. Callers route that
        // somewhere else -- there is nothing for the server to have an
        // opinion about -- so it must not arrive looking like a rejection.
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
