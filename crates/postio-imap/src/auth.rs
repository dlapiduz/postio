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
//! What deliberately does not live here: single-flight refresh and the
//! rejected-token → `Attention` routing (#194), and the OAuth authorization
//! flow itself (#192). This module is the seam they plug into.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::secret::{AccountKey, CommandSecretStore, Password, SecretError, SecretStore};

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
        }
    }
}

#[async_trait]
impl TokenSource for BrokerTokenSource {
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
        if let Some(token) = self.cache.lock().expect("token cache mutex").get(account) {
            return Ok(token.clone());
        }

        // Two concurrent misses run the broker twice; that is harmless
        // (brokers are idempotent reads) and single-flight is #194's
        // business, alongside the refresh stampede it exists for.
        let token = self.command.retrieve(account).await?;
        self.cache
            .lock()
            .expect("token cache mutex")
            .insert(account.clone(), token.clone());
        Ok(token)
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
}
