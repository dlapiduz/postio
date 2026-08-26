//! [`TokenSource`] for the flow this module builds.
//!
//! ADR 0006 Q1/Q5: one instance per account, refreshing on demand. The
//! refresh token lives in the keyring behind the same [`SecretStore`] a
//! password does — under a distinct [`AccountKey`] so an OAuth account and
//! a password account never collide — and the access token stays a
//! [`Password`] in memory, cached until shortly before it expires.
//!
//! What this type deliberately does not do: single-flight concurrent
//! refreshes, or route a rejected token to `Attention`. Both are #194.
//! `invalidate` here does the minimum that is still correct alone —
//! drop the cached access token, so the *next* call refreshes — and #194
//! adds the coalescing on top without changing this contract.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use url::Url;

use super::exchange::{self, RefreshExchange, TokenResponse};
use crate::auth::TokenSource;
use crate::cancel::CancelToken;
use crate::secret::{AccountKey, Password, SecretError, SecretStore};

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
    cache: Mutex<HashMap<AccountKey, CachedAccessToken>>,
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
    ) -> Self {
        Self {
            store,
            token_url,
            client_id: client_id.into(),
            client_secret,
            cache: Mutex::new(HashMap::new()),
        }
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
        self.cache.lock().expect("token cache mutex").insert(
            account.clone(),
            CachedAccessToken {
                token: response.access_token,
                expires_at: response.expires_in.map(|d| Instant::now() + d),
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
        let cancel = CancelToken::new();
        let response = exchange::refresh_token(
            &self.token_url,
            RefreshExchange {
                client_id: &self.client_id,
                client_secret: self.client_secret.as_deref(),
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
        self.cache.lock().expect("token cache mutex").insert(
            account.clone(),
            CachedAccessToken {
                token: response.access_token,
                expires_at: response.expires_in.map(|d| Instant::now() + d),
            },
        );
        Ok(access_token)
    }
}

#[async_trait]
impl TokenSource for OwnClientTokenSource {
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
        if let Some(cached) = self.cache.lock().expect("token cache mutex").get(account)
            && cached.is_fresh()
        {
            return Ok(cached.token.clone());
        }
        self.refresh(account).await
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

    fn account() -> AccountKey {
        AccountKey::new("ada@example.com")
    }

    /// A token endpoint that answers every request the same way, on a
    /// background thread, for as many requests as the test needs.
    fn mock_refresh_endpoint(body: &'static str) -> Url {
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
        let source = OwnClientTokenSource::new(store, url, "client-1", None);
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
        let source = OwnClientTokenSource::new(store, url, "client-1", None);
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
}
