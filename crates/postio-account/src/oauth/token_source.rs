//! ADR 0006 Q1/Q5: one per account. The refresh token lives in the keyring under a distinct
//! `AccountKey`; the access token stays in memory until shortly before expiry. Concurrent stale
//! callers share one refresh (`SingleFlight`) because a rotating provider burns the token the
//! second refresh presents.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use url::Url;

use super::exchange::{self, RefreshExchange, TokenResponse};
use crate::auth::TokenSource;
use crate::cancel::CancelToken;
use crate::secret::{AccountKey, Password, SecretError, SecretStore};
use crate::single_flight::SingleFlight;

/// Refreshing early keeps a caller from presenting a token that expired between the cache check and
/// the request reaching the server.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// The keyring holds one secret per `AccountKey`, and the refresh token is a different secret from
/// the password.
fn refresh_token_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-refresh", account.account()))
}

/// Google's "Desktop app" clients issue one and require it at the token endpoint even with PKCE.
fn client_secret_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-client-secret", account.account()))
}

/// Persisted so the account row can show "token valid 41d" after a restart, which a monotonic
/// `Instant` cannot survive (#870, #878). In the keyring, not `config.toml`, which strips
/// token-shaped keys.
fn oauth_expiry_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-expiry", account.account()))
}

fn refresh_deadline_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-refresh-deadline", account.account()))
}

/// Called at every renewal, because a sliding window resets on use and a deadline written once
/// would retire a working account (#954). Best-effort: losing it costs the early warning, never the
/// token.
async fn persist_refresh_deadline(
    store: &dyn SecretStore,
    account: &AccountKey,
    lifetime: Option<Duration>,
) {
    let key = refresh_deadline_key(account);
    let Some(lifetime) = lifetime else {
        // No stated lifetime clears: a row that drops the field must stop retiring accounts.
        let _ = store.delete(&key).await;
        return;
    };
    let Ok(since_epoch) = (SystemTime::now() + lifetime).duration_since(UNIX_EPOCH) else {
        return;
    };
    if let Err(error) = store
        .store(&key, &Password::new(since_epoch.as_secs().to_string()))
        .await
    {
        tracing::warn!(
            account = %account.account(),
            %error,
            "could not persist the OAuth refresh grant's deadline"
        );
    }
}

/// `None` means nothing to act on: no OAuth account, no grant yet, or no stated lifetime.
pub async fn stored_refresh_deadline(
    store: &dyn SecretStore,
    account: &AccountKey,
) -> Option<SystemTime> {
    let raw = store.retrieve(&refresh_deadline_key(account)).await.ok()?;
    let seconds: u64 = raw.expose().parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// Clears when there is nothing to persist, so a provider that stops saying `expires_in` leaves no
/// stale timestamp. Best-effort: a failure costs a wrong "valid for" line, not a broken login.
async fn persist_expiry(
    store: &dyn SecretStore,
    account: &AccountKey,
    expires_at: Option<Instant>,
) {
    let key = oauth_expiry_key(account);
    match expires_at {
        Some(at) => {
            // `Instant` has no fixed epoch; the wall-clock time is what a restarted process can
            // compare against.
            let wall_clock = SystemTime::now() + at.saturating_duration_since(Instant::now());
            let Ok(since_epoch) = wall_clock.duration_since(UNIX_EPOCH) else {
                return;
            };
            if let Err(error) = store
                .store(&key, &Password::new(since_epoch.as_secs().to_string()))
                .await
            {
                tracing::warn!(
                    account = %account.account(),
                    %error,
                    "could not persist the OAuth token's expiry"
                );
            }
        }
        // Silently best-effort: a provider that never sends `expires_in` would otherwise warn on
        // every refresh, and `SecretStore::delete` cannot tell "nothing there" from failure.
        None => {
            let _ = store.delete(&key).await;
        }
    }
}

/// `None` covers no OAuth account, no token minted, no `expires_in`, or an unopenable keyring; the
/// caller's answer is the same (#870, #878).
pub async fn stored_expiry(store: &dyn SecretStore, account: &AccountKey) -> Option<SystemTime> {
    let raw = store.retrieve(&oauth_expiry_key(account)).await.ok()?;
    let seconds: u64 = raw.expose().parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// Read back because a reconnect must present the same client as the first sign-in, and nobody is
/// there to ask for a credential registered months ago. `None` is the ordinary public-client case
/// and every failure besides.
pub async fn stored_client_secret(
    store: &dyn SecretStore,
    account: &AccountKey,
) -> Option<Password> {
    store.retrieve(&client_secret_key(account)).await.ok()
}

struct CachedAccessToken {
    token: Password,
    /// `None` means the server did not say: never stale on its own.
    expires_at: Option<Instant>,
}

impl CachedAccessToken {
    fn is_fresh(&self) -> bool {
        match self.expires_at {
            Some(at) => Instant::now() + EXPIRY_MARGIN < at,
            None => true,
        }
    }
}

pub struct OwnClientTokenSource {
    store: Arc<dyn SecretStore>,
    token_url: Url,
    client_id: String,
    client_secret: Option<String>,
    /// Looked up per account instead of carried, since nothing may hold a secret in a struct that
    /// lives for the process.
    stored_client_secret: bool,
    /// Carried from the account row so a refresh can renew the deadline without a lookup (#954).
    refresh_lifetime: Option<Duration>,
    cache: Mutex<HashMap<AccountKey, CachedAccessToken>>,
    /// A provider's tokens expire together, so a pool's sessions and the SMTP path find the cache
    /// stale at once: the stampede is the normal case (ADR 0006 Q5).
    refreshing: SingleFlight<AccountKey, Result<Password, SecretError>>,
}

impl std::fmt::Debug for OwnClientTokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnClientTokenSource")
            .field("token_url", &self.token_url.as_str())
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl OwnClientTokenSource {
    pub fn new(
        store: Arc<dyn SecretStore>,
        token_url: Url,
        client_id: impl Into<String>,
        client_secret: Option<String>,
        refresh_lifetime: Option<Duration>,
    ) -> Self {
        Self {
            store,
            token_url,
            client_id: client_id.into(),
            client_secret,
            stored_client_secret: false,
            refresh_lifetime,
            cache: Mutex::new(HashMap::new()),
            refreshing: SingleFlight::default(),
        }
    }

    /// The engine's constructor (#534): the client secret, when issued, is read from the keyring
    /// per refresh.
    pub fn with_stored_secret(
        store: Arc<dyn SecretStore>,
        token_url: Url,
        client_id: impl Into<String>,
        refresh_lifetime: Option<Duration>,
    ) -> Self {
        let mut source = Self::new(store, token_url, client_id, None, refresh_lifetime);
        source.stored_client_secret = true;
        source
    }

    pub async fn store_client_secret(
        &self,
        account: &AccountKey,
        secret: &Password,
    ) -> Result<(), SecretError> {
        self.store.store(&client_secret_key(account), secret).await
    }

    fn cached(&self, account: &AccountKey) -> Option<Password> {
        let cache = self.cache.lock().expect("token cache mutex");
        let cached = cache.get(account)?;
        cached.is_fresh().then(|| cached.token.clone())
    }

    /// A response with no refresh token leaves whatever the keyring already holds untouched.
    pub async fn seed(
        &self,
        account: &AccountKey,
        response: TokenResponse,
    ) -> Result<(), SecretError> {
        if let Some(refresh) = &response.refresh_token {
            self.store
                .store(&refresh_token_key(account), refresh)
                .await?;
        }
        let expires_at = response.expires_in.map(|d| Instant::now() + d);
        persist_expiry(self.store.as_ref(), account, expires_at).await;
        persist_refresh_deadline(self.store.as_ref(), account, self.refresh_lifetime).await;
        self.cache.lock().expect("token cache mutex").insert(
            account.clone(),
            CachedAccessToken {
                token: response.access_token,
                expires_at,
            },
        );
        Ok(())
    }

    async fn refresh(&self, account: &AccountKey) -> Result<Password, SecretError> {
        let refresh_token = self.store.retrieve(&refresh_token_key(account)).await?;

        // `access_token` carries no `CancelToken`: refresh is part of "give me a valid credential",
        // so it runs to `REQUEST_IO_TIMEOUT`.
        // A missing keyring entry means this client has no secret, the ordinary public-client case.
        let stored_secret = if self.stored_client_secret {
            self.store.retrieve(&client_secret_key(account)).await.ok()
        } else {
            None
        };
        let client_secret = stored_secret
            .as_ref()
            .map(|secret| secret.expose())
            .or(self.client_secret.as_deref());

        let cancel = CancelToken::new();
        let response = exchange::refresh_token(
            &self.token_url,
            RefreshExchange {
                client_id: &self.client_id,
                client_secret,
                refresh_token: refresh_token.expose(),
            },
            &cancel,
        )
        .await
        .map_err(|err| SecretError::Backend {
            account: account.account().to_string(),
            reason: err.to_string(),
        })?;

        // Some providers rotate the refresh token on every use: persist it before returning, or the
        // next refresh presents a burned one.
        if let Some(rotated) = &response.refresh_token {
            self.store
                .store(&refresh_token_key(account), rotated)
                .await?;
        }

        let access_token = response.access_token.clone();
        let expires_at = response.expires_in.map(|d| Instant::now() + d);
        persist_expiry(self.store.as_ref(), account, expires_at).await;
        // Unconditionally: a sliding window resets on use, so an account that keeps refreshing must
        // never reach its deadline.
        persist_refresh_deadline(self.store.as_ref(), account, self.refresh_lifetime).await;
        self.cache.lock().expect("token cache mutex").insert(
            account.clone(),
            CachedAccessToken {
                token: response.access_token,
                expires_at,
            },
        );
        Ok(access_token)
    }
}

#[async_trait]
impl TokenSource for OwnClientTokenSource {
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
        if let Some(token) = self.cached(account) {
            return Ok(token);
        }
        // Before the round trip, so a dead grant says so without the user watching a sync fail
        // (#954). A cached access token is still served: it works until it expires.
        if let Some(deadline) = stored_refresh_deadline(self.store.as_ref(), account).await
            && SystemTime::now() >= deadline
        {
            return Err(SecretError::GrantExpired {
                account: account.account().to_string(),
            });
        }
        self.refreshing
            .run(account, async {
                // Checked again inside the flight: a caller arriving as the previous refresh lands
                // takes its token.
                if let Some(token) = self.cached(account) {
                    return Ok(token);
                }
                self.refresh(account).await
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    use crate::secret::MemorySecretStore;

    use super::*;

    pub(super) fn account() -> AccountKey {
        AccountKey::new("ada@example.com")
    }

    pub(super) fn mock_refresh_endpoint(body: &'static str) -> Url {
        mock_refresh_endpoint_counting(body).0
    }

    pub(super) fn mock_refresh_endpoint_counting(
        body: &'static str,
    ) -> (Url, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (endpoint(body, std::sync::Arc::clone(&served)), served)
    }

    fn endpoint(body: &'static str, served: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let mut content_length = 0usize;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).expect("header line");
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut discard = vec![0u8; content_length];
                let _ = std::io::Read::read_exact(&mut reader, &mut discard);
                served.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        format!("http://127.0.0.1:{port}/token").parse().unwrap()
    }

    #[tokio::test]
    async fn a_seeded_access_token_is_returned_without_a_refresh() {
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(
            store,
            "http://127.0.0.1:1/token".parse().unwrap(),
            "client-1",
            None,
            None,
        );
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("seeded-token"),
                    refresh_token: Some(Password::new("seeded-refresh")),
                    expires_in: Some(Duration::from_secs(3600)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        let token = source.access_token(&account()).await.expect("cached token");
        assert_eq!(token.expose(), "seeded-token");
    }

    #[tokio::test]
    async fn an_expired_cached_token_is_refreshed_from_the_stored_refresh_token() {
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed-token","expires_in":3600}"#);
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(store, url, "client-1", None, None);
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("stale"),
                    refresh_token: Some(Password::new("the-refresh-token")),
                    expires_in: Some(Duration::from_secs(0)), // already stale
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        let token = source
            .access_token(&account())
            .await
            .expect("refresh succeeds");
        assert_eq!(token.expose(), "refreshed-token");
    }

    #[tokio::test]
    async fn invalidate_forces_the_next_call_to_refresh() {
        let url = mock_refresh_endpoint(r#"{"access_token":"after-invalidate","expires_in":3600}"#);
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(store, url, "client-1", None, None);
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("before-invalidate"),
                    refresh_token: Some(Password::new("the-refresh-token")),
                    expires_in: Some(Duration::from_secs(3600)), // still fresh
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        source.invalidate(&account()).await;

        let token = source
            .access_token(&account())
            .await
            .expect("refresh succeeds");
        assert_eq!(token.expose(), "after-invalidate");
    }

    #[tokio::test]
    async fn a_rotated_refresh_token_replaces_the_stored_one() {
        let url = mock_refresh_endpoint(
            r#"{"access_token":"a","expires_in":3600,"refresh_token":"rotated"}"#,
        );
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(
            Arc::clone(&store) as Arc<dyn SecretStore>,
            url,
            "client-1",
            None,
            None,
        );
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("stale"),
                    refresh_token: Some(Password::new("original")),
                    expires_in: Some(Duration::from_secs(0)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        source
            .access_token(&account())
            .await
            .expect("refresh succeeds");

        let stored = store
            .retrieve(&refresh_token_key(&account()))
            .await
            .expect("stored");
        assert_eq!(stored.expose(), "rotated");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_callers_during_expiry_cause_exactly_one_refresh() {
        let (url, served) =
            mock_refresh_endpoint_counting(r#"{"access_token":"one-refresh","expires_in":3600}"#);
        let store = Arc::new(MemorySecretStore::new());
        let source = Arc::new(OwnClientTokenSource::new(
            store, url, "client-1", None, None,
        ));
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("stale"),
                    refresh_token: Some(Password::new("the-refresh-token")),
                    expires_in: Some(Duration::from_secs(0)), // already stale
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let source = Arc::clone(&source);
            tasks.spawn(async move { source.access_token(&account()).await });
        }
        let mut tokens = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            tokens.push(joined.expect("the task should not panic").expect("a token"));
        }

        assert_eq!(
            served.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "six callers, one round trip to the token endpoint"
        );
        assert!(
            tokens.iter().all(|token| token.expose() == "one-refresh"),
            "and all six got the token that refresh produced"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_refresh_that_fails_is_also_only_attempted_once() {
        let (url, served) = mock_refresh_endpoint_counting(r#"{"not":"a token response"}"#);
        let store = Arc::new(MemorySecretStore::new());
        let source = Arc::new(OwnClientTokenSource::new(
            store, url, "client-1", None, None,
        ));
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("stale"),
                    refresh_token: Some(Password::new("the-refresh-token")),
                    expires_in: Some(Duration::from_secs(0)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..5 {
            let source = Arc::clone(&source);
            tasks.spawn(async move { source.access_token(&account()).await });
        }
        let mut refused = 0;
        while let Some(joined) = tasks.join_next().await {
            assert!(joined.expect("the task should not panic").is_err());
            refused += 1;
        }

        assert_eq!(refused, 5, "every caller was told");
        assert_eq!(
            served.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "and the endpoint was asked once"
        );
    }

    #[tokio::test]
    async fn seeding_with_an_expiry_makes_it_readable_afterwards() {
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(
            store.clone(),
            "http://127.0.0.1:1/token".parse().unwrap(),
            "client-1",
            None,
            None,
        );
        let before = SystemTime::now();
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("seeded-token"),
                    refresh_token: Some(Password::new("seeded-refresh")),
                    expires_in: Some(Duration::from_secs(3600)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        let expiry = stored_expiry(store.as_ref(), &account())
            .await
            .expect("an expiry was persisted");
        let elapsed = expiry
            .duration_since(before)
            .expect("the stored expiry is not before the call that produced it");
        assert!(
            (3595..=3605).contains(&elapsed.as_secs()),
            "expected roughly 3600s out, got {}s",
            elapsed.as_secs()
        );
    }

    #[tokio::test]
    async fn a_token_response_with_no_expires_in_leaves_nothing_to_read() {
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(
            store.clone(),
            "http://127.0.0.1:1/token".parse().unwrap(),
            "client-1",
            None,
            None,
        );
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("seeded-token"),
                    refresh_token: Some(Password::new("seeded-refresh")),
                    expires_in: None,
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        assert!(
            stored_expiry(store.as_ref(), &account()).await.is_none(),
            "a provider that never said expires_in has nothing to show as a validity"
        );
    }

    #[tokio::test]
    async fn a_second_grant_with_no_expiry_clears_the_first_ones() {
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(
            store.clone(),
            "http://127.0.0.1:1/token".parse().unwrap(),
            "client-1",
            None,
            None,
        );
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("first"),
                    refresh_token: Some(Password::new("refresh-1")),
                    expires_in: Some(Duration::from_secs(3600)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");
        assert!(stored_expiry(store.as_ref(), &account()).await.is_some());

        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("second"),
                    refresh_token: Some(Password::new("refresh-2")),
                    expires_in: None,
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");

        assert!(
            stored_expiry(store.as_ref(), &account()).await.is_none(),
            "a stale expiry from the earlier grant must not outlive it"
        );
    }

    #[tokio::test]
    async fn a_refresh_updates_the_persisted_expiry_too() {
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed","expires_in":60}"#);
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(store.clone(), url, "client-1", None, None);
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("stale"),
                    refresh_token: Some(Password::new("the-refresh-token")),
                    expires_in: Some(Duration::from_secs(0)), // already stale
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");
        let seeded_expiry = stored_expiry(store.as_ref(), &account()).await;

        source
            .access_token(&account())
            .await
            .expect("refresh succeeds");

        let refreshed_expiry = stored_expiry(store.as_ref(), &account())
            .await
            .expect("the refresh persisted its own expiry");
        assert!(
            refreshed_expiry >= seeded_expiry.unwrap_or(UNIX_EPOCH),
            "the refreshed expiry (60s out) must not read as earlier than the \
             stale one it replaced (already past)"
        );
    }

    #[tokio::test]
    async fn the_expiry_survives_a_fresh_token_source_over_the_same_store() {
        // `Instant` cannot cross a restart, so a second source standing in for a fresh launch must
        // read what the first wrote.
        let store = Arc::new(MemorySecretStore::new());
        let first = OwnClientTokenSource::new(
            store.clone(),
            "http://127.0.0.1:1/token".parse().unwrap(),
            "client-1",
            None,
            None,
        );
        first
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("seeded-token"),
                    refresh_token: Some(Password::new("seeded-refresh")),
                    expires_in: Some(Duration::from_secs(3600)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");
        drop(first);

        assert!(
            stored_expiry(store.as_ref(), &account()).await.is_some(),
            "a fresh source over the same keyring must still see the earlier one's expiry"
        );
    }
}

#[cfg(test)]
mod refresh_deadline_tests {
    use std::sync::atomic::Ordering;

    use crate::secret::MemorySecretStore;

    use super::tests::{account, mock_refresh_endpoint, mock_refresh_endpoint_counting};
    use super::*;

    async fn seeded(
        lifetime: Option<Duration>,
        url: Url,
    ) -> (Arc<MemorySecretStore>, OwnClientTokenSource) {
        let store = Arc::new(MemorySecretStore::new());
        let source = OwnClientTokenSource::new(store.clone(), url, "client-1", None, lifetime);
        source
            .seed(
                &account(),
                TokenResponse {
                    access_token: Password::new("stale"),
                    refresh_token: Some(Password::new("the-refresh-token")),
                    expires_in: Some(Duration::from_secs(0)),
                    token_type: "Bearer".to_string(),
                    scope: None,
                },
            )
            .await
            .expect("seed succeeds");
        (store, source)
    }

    #[tokio::test]
    async fn minting_a_grant_records_when_it_dies() {
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed","expires_in":60}"#);
        let (store, _source) = seeded(Some(Duration::from_secs(7 * 86_400)), url).await;

        let deadline = stored_refresh_deadline(store.as_ref(), &account())
            .await
            .expect("the mint recorded a deadline");
        let expected = SystemTime::now() + Duration::from_secs(7 * 86_400);
        let slack = Duration::from_secs(60);
        assert!(
            deadline > expected - slack && deadline < expected + slack,
            "a seven-day grant should die in about seven days"
        );
    }

    #[tokio::test]
    async fn a_provider_that_states_no_lifetime_records_no_deadline() {
        // The must-not-regress case: absent means absent, not zero.
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed","expires_in":60}"#);
        let (store, _source) = seeded(None, url).await;
        assert!(
            stored_refresh_deadline(store.as_ref(), &account())
                .await
                .is_none(),
            "a provider with no stated lifetime must leave no deadline behind"
        );
    }

    #[tokio::test]
    async fn every_refresh_pushes_the_deadline_out() {
        // Sliding windows reset on use: persisting only at mint would mark a healthy account stale.
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed","expires_in":0}"#);
        let (store, source) = seeded(Some(Duration::from_secs(90 * 86_400)), url).await;
        assert!(
            stored_refresh_deadline(store.as_ref(), &account())
                .await
                .is_some(),
            "the mint recorded one to begin with"
        );

        // Cleared, so what is found can only have been written by the refresh: both writes land in
        // the same second.
        store
            .delete(&refresh_deadline_key(&account()))
            .await
            .expect("clearing the deadline");

        source
            .access_token(&account())
            .await
            .expect("the refresh succeeds");

        assert!(
            stored_refresh_deadline(store.as_ref(), &account())
                .await
                .is_some(),
            "a refresh must renew the deadline, or a sliding window retires \
             the account that is using it most"
        );
    }

    #[tokio::test]
    async fn a_grant_past_its_deadline_is_refused_without_asking_the_server() {
        let (url, served) =
            mock_refresh_endpoint_counting(r#"{"access_token":"refreshed","expires_in":60}"#);
        let (_store, source) = seeded(Some(Duration::from_secs(0)), url).await;

        let error = source
            .access_token(&account())
            .await
            .expect_err("a dead grant cannot produce a token");
        assert!(
            matches!(error, SecretError::GrantExpired { .. }),
            "expected a dead grant, got {error:?}"
        );
        assert_eq!(
            served.load(Ordering::SeqCst),
            0,
            "a grant known to be dead must not cost a round trip"
        );
    }

    #[tokio::test]
    async fn a_grant_with_no_deadline_still_refreshes_exactly_as_before() {
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed","expires_in":60}"#);
        let (_store, source) = seeded(None, url).await;
        let token = source
            .access_token(&account())
            .await
            .expect("no deadline, so nothing refuses it");
        assert_eq!(token.expose(), "refreshed");
    }
}
