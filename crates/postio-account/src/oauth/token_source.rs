//! [`TokenSource`] for the flow this module builds.
//!
//! ADR 0006 Q1/Q5: one instance per account, refreshing on demand. The
//! refresh token lives in the keyring behind the same [`SecretStore`] a
//! password does — under a distinct [`AccountKey`] so an OAuth account and
//! a password account never collide — and the access token stays a
//! [`Password`] in memory, cached until shortly before it expires.
//!
//! Concurrent callers finding the same token stale share one refresh
//! ([`SingleFlight`], ADR 0006 Q5): on a provider that rotates its refresh
//! token on every use, the second and third simultaneous refresh present a
//! token the server has already burned. `invalidate` drops the cached access
//! token so the *next* call refreshes; routing a rejection to the user is the
//! session layer's job, at the layer that saw the server say no.

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

/// How much earlier than its stated expiry a cached access token is treated
/// as stale. A refresh takes a round trip; starting it before the token is
/// actually rejected is what keeps a caller from ever presenting one that
/// expired between the cache check and the request reaching the server.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// The keyring holds one secret per [`AccountKey`] ([`SecretStore`]'s own
/// contract), and an account's password and its OAuth refresh token are two
/// different secrets — so the refresh token is stored under a derived key
/// rather than the account's own, which stays free for
/// [`crate::auth::StoredPasswordSource`] to mean what it already means.
fn refresh_token_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-refresh", account.account()))
}

/// Where an account's own OAuth client secret lives, when its provider
/// issued one — Google's "Desktop app" clients do, and require it at the
/// token endpoint even with PKCE. Same derived-key discipline as the
/// refresh token: a distinct secret under a distinct key.
fn client_secret_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-client-secret", account.account()))
}

/// Where an account's access-token expiry lives, alongside its refresh
/// token — the settings redesign's own account row (#878) wants to show
/// "token valid 41d" / "token expired", which needs this to survive past
/// the process that minted it, and `CachedAccessToken::expires_at`'s
/// [`Instant`] does not: a monotonic clock has no fixed epoch, so it
/// cannot be written down and read back after a restart.
///
/// Alongside the credential in the keyring, not in `config.toml` (#870,
/// per the maintainer): `crates/postio-config/src/secrets.rs` already
/// strips anything password/token/secret-shaped out of that file on the
/// way in and out, and an expiry timestamp naming exactly when a token
/// (and, by the same shape, the account) is good for is the kind of thing
/// that rule exists to keep off disk in plain text next to everything
/// else config.toml holds.
///
/// Not a real secret — a timestamp is not sensitive the way a token is —
/// but the keyring is where every other derived fact about this
/// credential already lives, and a second storage mechanism for one
/// string would be a second thing to keep working, not a smaller one.
fn oauth_expiry_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-expiry", account.account()))
}

/// Where the **refresh** grant's own deadline is kept.
///
/// Beside the access token's expiry and for the same reasons — the keyring
/// is where every derived fact about this credential already lives, and
/// `config.toml` strips anything token-shaped on the way through.
fn refresh_deadline_key(account: &AccountKey) -> AccountKey {
    AccountKey::new(format!("{}#oauth-refresh-deadline", account.account()))
}

/// Records when `account`'s refresh grant dies, or clears the record when
/// the provider states no lifetime.
///
/// Called at every point the grant is renewed — the mint and every refresh —
/// because a sliding window resets on use, and a deadline written only once
/// would retire an account that is working perfectly (#954).
///
/// Best-effort, exactly like [`persist_expiry`]: losing this costs the early
/// warning, never the token. The reactive path still catches a dead grant
/// the way it always did.
async fn persist_refresh_deadline(
    store: &dyn SecretStore,
    account: &AccountKey,
    lifetime: Option<Duration>,
) {
    let key = refresh_deadline_key(account);
    let Some(lifetime) = lifetime else {
        // No stated lifetime clears rather than leaves: a provider row that
        // drops the field must stop retiring accounts, not keep acting on a
        // deadline nobody stands behind any more.
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

/// When `account`'s refresh grant dies, if its provider states a lifetime.
///
/// `None` means there is nothing to act on — no OAuth account, no grant yet,
/// or a provider that states no lifetime — and every one of those must behave
/// as Postio did before this existed.
pub async fn stored_refresh_deadline(
    store: &dyn SecretStore,
    account: &AccountKey,
) -> Option<SystemTime> {
    let raw = store.retrieve(&refresh_deadline_key(account)).await.ok()?;
    let seconds: u64 = raw.expose().parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// Persists `expires_at` for `account`, or clears whatever was there when
/// there is nothing to persist — a provider that stops saying `expires_in`
/// must not leave a stale timestamp behind for the account row to read as
/// current.
///
/// Best-effort: a store this fails on already logged why through `store`'s
/// own error path when the refresh token next needs it, and losing the
/// expiry costs a wrong "valid for" line in a settings pane, not a broken
/// login — worth a warning, not worth failing the refresh that got a
/// caller a working token.
async fn persist_expiry(
    store: &dyn SecretStore,
    account: &AccountKey,
    expires_at: Option<Instant>,
) {
    let key = oauth_expiry_key(account);
    match expires_at {
        Some(at) => {
            // `Instant` has no fixed epoch; the wall-clock instant this
            // many seconds from now, on the other hand, is exactly what a
            // later, possibly-restarted process needs to compare itself
            // against.
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
        // Best-effort, and silently so: a provider that never sends
        // `expires_in` clears nothing on every single refresh, which
        // would otherwise warn on every one of them for having nothing to
        // clear — `SecretStore::delete` has no "there was nothing there"
        // variant distinct from a real failure to tell those apart by.
        None => {
            let _ = store.delete(&key).await;
        }
    }
}

/// The persisted expiry for `account`'s OAuth access token, if
/// [`OwnClientTokenSource`] has ever recorded one — what an account row
/// reads to show "token valid 41d" without holding a live token source of
/// its own (#870, #878).
///
/// `None` covers every reason there is nothing to show: no OAuth account
/// under this key, no token minted yet, a provider that never said
/// `expires_in`, or a keyring this call cannot open — the caller's answer
/// is the same either way, since there is nothing here to act on it (a
/// genuinely locked keyring is loud enough elsewhere, on the read that
/// actually needs the credential).
pub async fn stored_expiry(store: &dyn SecretStore, account: &AccountKey) -> Option<SystemTime> {
    let raw = store.retrieve(&oauth_expiry_key(account)).await.ok()?;
    let seconds: u64 = raw.expose().parse().ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

struct CachedAccessToken {
    token: Password,
    /// `None` means "the server did not say", treated as never stale on
    /// its own — a caller only learns such a token is bad by
    /// [`TokenSource::invalidate`], same as [`crate::auth::StoredPasswordSource`].
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

/// The user's own OAuth client, refreshing against a token endpoint.
///
/// ADR 0006 Q1 — `OwnClientTokenSource`: the user's own `client_id` (and,
/// rarely, `client_secret`) from their own cloud project. No verification
/// burden, because it is the user's own project rather than Postio's.
pub struct OwnClientTokenSource {
    store: Arc<dyn SecretStore>,
    token_url: Url,
    client_id: String,
    client_secret: Option<String>,
    /// Look the client secret up in the keyring per account instead of
    /// carrying it: the shape the engine rebuilds at every launch, where
    /// nothing may hold a secret in a struct that lives for the process.
    stored_client_secret: bool,
    /// How long this provider says its refresh grant lives, when it says.
    ///
    /// Provider data, carried here from the account row so a refresh can
    /// renew the deadline without a lookup — and `None` everywhere the
    /// provider states nothing, which is the case that must behave exactly
    /// as Postio did before #954.
    refresh_lifetime: Option<Duration>,
    cache: Mutex<HashMap<AccountKey, CachedAccessToken>>,
    /// One refresh per account at a time, its result shared.
    ///
    /// An OAuth provider's tokens expire together, so the pool's sessions and
    /// the SMTP path all find the cache stale at once — the stampede is the
    /// normal case here, not the edge (ADR 0006 Q5).
    refreshing: SingleFlight<AccountKey, Result<Password, SecretError>>,
}

impl std::fmt::Debug for OwnClientTokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the client secret, and never a cached access token — same
        // discipline `BrokerTokenSource`'s `Debug` follows.
        f.debug_struct("OwnClientTokenSource")
            .field("token_url", &self.token_url.as_str())
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl OwnClientTokenSource {
    /// Builds a source refreshing against `token_url` for `client_id`,
    /// storing and reading refresh tokens through `store`.
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

    /// The engine's constructor (#534): the client id and endpoint come
    /// from the account row, and the client secret — when the provider
    /// issued one at sign-in — is read from the keyring per refresh,
    /// under its own derived key.
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

    /// Stores the client secret the sign-in flow was given, so
    /// [`with_stored_secret`](Self::with_stored_secret) finds it at every
    /// later launch.
    pub async fn store_client_secret(
        &self,
        account: &AccountKey,
        secret: &Password,
    ) -> Result<(), SecretError> {
        self.store.store(&client_secret_key(account), secret).await
    }

    /// The cached access token for `account`, if there is one and it is not
    /// about to expire.
    fn cached(&self, account: &AccountKey) -> Option<Password> {
        let cache = self.cache.lock().expect("token cache mutex");
        let cached = cache.get(account)?;
        cached.is_fresh().then(|| cached.token.clone())
    }

    /// Records the tokens a completed [`super::authorize`] attempt
    /// produced: the refresh token goes to the keyring, the access token
    /// primes the in-memory cache so the very next call does not refresh a
    /// token that was just minted.
    ///
    /// A response with no refresh token (a provider that issues
    /// access-token-only grants, or a refresh already on file) leaves
    /// whatever the keyring already holds untouched.
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

        // `TokenSource::access_token` carries no `CancelToken` — refreshing
        // is an implementation detail of "give me a valid credential", not
        // a user-visible wait with a Cancel button, so this request runs to
        // completion or to `REQUEST_IO_TIMEOUT`'s own bound rather than a
        // caller-supplied one.
        // The static secret when construction carried one; the keyring's
        // when the engine asked for the stored shape; a missing keyring
        // entry is simply "this client has no secret", which is the
        // ordinary public-client case.
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

        // Some providers rotate the refresh token on every use and
        // invalidate the previous one — persist whatever they handed back
        // before returning, or the *next* refresh would present a token
        // the server already burned.
        if let Some(rotated) = &response.refresh_token {
            self.store
                .store(&refresh_token_key(account), rotated)
                .await?;
        }

        let access_token = response.access_token.clone();
        let expires_at = response.expires_in.map(|d| Instant::now() + d);
        persist_expiry(self.store.as_ref(), account, expires_at).await;
        // Unconditionally, and not only when the provider rotated the token:
        // a sliding window resets on *use*, so the account that keeps
        // refreshing is exactly the one that must never reach its deadline.
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
        // Before the round trip, not after it. The deadline is known here,
        // so an account whose grant has died says so without the user first
        // watching a sync fail (#954). A cached access token is still served
        // above: it works until it expires, and refusing it early would
        // interrupt a session for a deadline that has not cost anything yet.
        if let Some(deadline) = stored_refresh_deadline(self.store.as_ref(), account).await
            && SystemTime::now() >= deadline
        {
            return Err(SecretError::GrantExpired {
                account: account.account().to_string(),
            });
        }
        self.refreshing
            .run(account, async {
                // Checked again inside the flight. A caller that arrived in
                // the moment the previous refresh landed leads a flight of
                // its own, and it should take the token that refresh just
                // produced rather than spend a second round trip finding out
                // it exists.
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

    /// A token endpoint that answers every request the same way, on a
    /// background thread, for as many requests as the test needs.
    pub(super) fn mock_refresh_endpoint(body: &'static str) -> Url {
        mock_refresh_endpoint_counting(body).0
    }

    /// As [`mock_refresh_endpoint`], handing back the count of requests it has
    /// served — which is the only place "how many refreshes happened" can
    /// honestly be observed.
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
        // No server is even started: if this reaches the network the test
        // hangs or errors, proving the cache was actually consulted.
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

    // -----------------------------------------------------------------------
    // The stampede — ADR 0006 Q5, #194
    // -----------------------------------------------------------------------

    /// The acceptance criterion, stated where it can be counted: a provider's
    /// access tokens expire together, so a pool's sessions and the SMTP path
    /// all find the cache stale within the same second. The token endpoint
    /// must see one request.
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

    /// The half a mutex would get wrong. A revoked grant answers every waiter
    /// from the one attempt rather than letting each take its turn at an
    /// endpoint that is already saying no.
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

    // -- Acceptance: the expiry survives past the process that minted it (#870) --

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
        // 3600s from `before`, give or take the time this test itself took
        // and the second the stored value is truncated to.
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
        // The property this whole issue is about: `Instant` cannot cross a
        // process restart, so a second `OwnClientTokenSource` -- standing in
        // for a fresh launch -- must still be able to read what the first
        // one wrote.
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

    /// A grant that was just minted, for a provider that states `lifetime`.
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
                    // Already stale, so the next call has to refresh.
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
        // Microsoft's ninety days slide: they reset on each use, so an
        // account in continuous use must never reach its deadline. Persisting
        // only at mint is what would mark a healthy account stale.
        let url = mock_refresh_endpoint(r#"{"access_token":"refreshed","expires_in":0}"#);
        let (store, source) = seeded(Some(Duration::from_secs(90 * 86_400)), url).await;
        assert!(
            stored_refresh_deadline(store.as_ref(), &account())
                .await
                .is_some(),
            "the mint recorded one to begin with"
        );

        // Cleared, so what is found afterwards can only have been written by
        // the refresh. Comparing timestamps instead would prove nothing: both
        // writes land in the same second, so a refresh that never wrote leaves
        // the mint's deadline behind and every `>=` still holds.
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
        // The whole point: the user learns before a sync is attempted, and
        // the server is never troubled for a grant that cannot work.
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
