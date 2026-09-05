//! Building the sync engine for an account.
//!
//! Everything here is a choice about *this* installation rather than about
//! how mail works: which TLS stack dials the server, where the password is
//! kept, how many connections to hold open. That is what a composition root
//! is for, and it is why `postio-runtime` takes these as parts rather than
//! constructing them — a test hands it a mock backend and an in-memory
//! keyring through the same door.
//!
//! # Nothing here dials anything
//!
//! `ImapBackend::new` builds a pool and opens no connection; the engine's
//! supervisor decides when to. So a Postio started with no network, or with
//! a password the keyring will not give up, still opens its window and says
//! so — which is the whole point of being local-first.
//!
//! [`NetworkSource::NetworkManager`] is asked for here rather than defaulted
//! to inside the engine, because a listener that opened a system-bus
//! connection on its own would make every test behave differently on a
//! desktop and on a CI runner. A machine without NetworkManager loses nothing
//! but the promptness of a reconnect.

use std::sync::Arc;

use postio_account::auth::{StoredPasswordSource, TokenSource};
use postio_account::backend::MailBackend;
use postio_account::imap::{
    ConnectionPool, ConnectionSettings, ImapBackend, PoolConfig, RustlsConnector,
};
use postio_account::secret::{AccountKey, SecretStore};
use postio_model::{Account, AccountId};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_storage::{BlobStore, Database};

use postio_core::bridge::EventSink;

/// Start the engine for `account`.
///
/// `None` when the transports cannot be built at all — a system with no
/// usable TLS stack, say. That costs the account its sync and nothing else:
/// the local store still opens and everything already synced still reads.
///
/// `secrets` is handed in rather than built here for the reason the module
/// docs give: it is the composition root's choice, and it is the same store
/// onboarding writes the password into and startup reads it back from.
// Nine parts because the composition root chooses all nine — the module
// docs' whole argument. `start_joining` below already carries the allow for
// the same reason.
#[allow(clippy::too_many_arguments)]
pub fn start(
    account: &Account,
    database: &Database,
    blobs: BlobStore,
    events: EventSink,
    secrets: Arc<dyn SecretStore>,
    mailbox_roles: postio_model::RoleOverrides,
    rules: postio_search::rules::RuleSet,
    backfill: postio_runtime::BackfillPolicy,
    watch: postio_sync::WatchPolicy,
    egress: Arc<dyn postio_model::egress::EgressSink>,
) -> Option<Engine> {
    let key = AccountKey::new(account.address.address.clone());

    // Both transports report to the egress log (#151): every connection
    // this engine opens is a row the user can audit.
    let connector = match RustlsConnector::new() {
        Ok(connector) => Arc::new(connector.with_egress(egress.clone())),
        Err(error) => {
            tracing::error!(%error, "no IMAP transport, so no sync");
            return None;
        }
    };
    let smtp = match postio_smtp::transport::RustlsConnector::new() {
        Ok(connector) => Arc::new(connector.with_egress(egress)),
        Err(error) => {
            tracing::error!(%error, "no SMTP transport, so nothing can be sent");
            return None;
        }
    };

    // One `TokenSource` for this account, and both sides of it get *this*
    // instance (ADR 0006 Q5). A second source built for SMTP would look
    // identical and would be the bug: a rejection seen while fetching would
    // be invisible while sending, and two simultaneous refreshes on a
    // provider that rotates its refresh token invalidate each other.
    //
    // A password account is a `TokenSource` too. That is the point of the
    // seam — the composition root chooses which kind of credential this
    // account has, and nothing downstream asks again.
    let tokens = token_source(account, &secrets);

    let backend = backend_for(account, key, tokens.clone(), connector);

    match Engine::spawn(EngineParts {
        account: account.id,
        database: database.clone(),
        blobs,
        backend,
        smtp,
        tokens,
        events,
        retry: Default::default(),
        backfill,
        reconnect: Default::default(),
        watch,
        network: NetworkSource::NetworkManager,
        mailbox_roles,
        rules,
        clock: Arc::new(SystemClock),
    }) {
        Ok(engine) => Some(engine),
        Err(error) => {
            tracing::error!(%error, "the sync engine did not start");
            None
        }
    }
}

/// The adapter the account's stored backend choice names (#545, ADR 0018
/// Q5): the one place a protocol is picked, so nothing downstream asks
/// again. A `jmap` row whose stored session URL no longer parses falls
/// back to the IMAP adapter with an error in the log — the incoming
/// server is stored either way, and a degraded sync beats none.
fn backend_for(
    account: &Account,
    key: AccountKey,
    tokens: Arc<dyn postio_account::auth::TokenSource>,
    connector: Arc<RustlsConnector>,
) -> Arc<dyn MailBackend> {
    match &account.backend {
        postio_model::account::Backend::Jmap { session_url } => match session_url.parse() {
            Ok(url) => {
                return Arc::new(postio_jmap::JmapBackend::with_token_source(
                    url, key, tokens,
                ));
            }
            Err(error) => {
                tracing::error!(
                    account = account.id.get(),
                    %error,
                    "the stored JMAP session URL does not parse; falling back to IMAP"
                );
            }
        },
        postio_model::account::Backend::Gmail => {
            return Arc::new(postio_gmail::GmailBackend::with_token_source(key, tokens));
        }
        postio_model::account::Backend::Imap => {}
    }
    Arc::new(ImapBackend::over(Arc::new(
        ConnectionPool::with_token_source(
            settings(&account.incoming, account.auth),
            key,
            tokens,
            connector,
            PoolConfig::default(),
        ),
    )))
}

/// The account's IMAP server, as the connection pool wants it.
///
/// Field for field. The two types exist separately because `postio-model`
/// describes an account and `postio-account` describes a connection, and neither
/// should have to change when the other does.
/// Which strategy obtains this account's credential (ADR 0006 Q1, #534).
///
/// The account's own data decides: an [`OAuthConfig`] on the row means the
/// sign-in flow ran with the user's own client, and refreshes go through
/// it — the client secret, when one exists, read from the keyring under
/// its derived key. No config means the keyring entry *is* the credential:
/// a stored password, an app password, or a broker-minted token that an
/// external tool keeps fresh — all the same shape to the sessions.
///
/// [`OAuthConfig`]: postio_model::account::OAuthConfig
pub(crate) fn token_source(
    account: &Account,
    secrets: &Arc<dyn SecretStore>,
) -> Arc<dyn TokenSource> {
    if let Some(oauth) = &account.oauth {
        match oauth.token_url.parse() {
            Ok(token_url) => {
                return Arc::new(
                    postio_account::oauth::OwnClientTokenSource::with_stored_secret(
                        secrets.clone(),
                        token_url,
                        oauth.client_id.clone(),
                    ),
                );
            }
            Err(error) => {
                // A row this build cannot parse must not cost the account
                // its mail: the keyring may still hold a workable token,
                // and saying so beats an engine that never starts.
                tracing::error!(
                    %error,
                    "the account's stored OAuth token endpoint is not a URL; \
                     falling back to the stored credential"
                );
            }
        }
    }
    Arc::new(StoredPasswordSource::new(secrets.clone()))
}

pub(crate) fn settings(
    server: &postio_model::account::ServerConfig,
    auth: postio_model::account::AuthMethod,
) -> ConnectionSettings {
    let mut settings = ConnectionSettings::new(
        server.host.clone(),
        server.port,
        server.security,
        server.username.clone(),
    );
    // The stored mechanism, carried through — dropping it here is #533:
    // every account authenticated as Password however it was stored, and
    // the OAUTHBEARER/XOAUTH2 paths #193 wired in were reachable from
    // tests only.
    settings.auth = auth;
    settings
}

// ---------------------------------------------------------------------------
// One engine per enabled account
// ---------------------------------------------------------------------------

/// Why the root refused to start the engines it was asked for.
///
/// A refusal rather than a silent truncation: an account whose engine never
/// started is an account whose mail silently stops arriving, and "some of your
/// mail is syncing" is the kind of half-state a mail client must never be in
/// without saying so (#183).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupRefusal {
    /// More enabled accounts than the connection pool can serve at once.
    TooManyAccounts {
        /// How many enabled accounts were found.
        accounts: usize,
        /// How many engines this pool can carry.
        budget: usize,
    },
}

impl std::fmt::Display for StartupRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupRefusal::TooManyAccounts { accounts, budget } => write!(
                f,
                "{accounts} accounts are enabled but this database pool can \
                 serve {budget} at once. Disable an account, or raise the \
                 pool size, and restart."
            ),
        }
    }
}

impl std::error::Error for StartupRefusal {}

/// How many engines a pool of `max_connections` can carry.
///
/// One connection per engine, and one left for the frontend — which reads on
/// every repaint and must never be the caller that waits. Saturating rather
/// than panicking on a one-connection pool: the answer there is "no engines",
/// which [`start_all`] reports as a refusal with a sentence rather than
/// starting one and deadlocking against the UI.
pub fn engine_budget(max_connections: usize) -> usize {
    max_connections.saturating_sub(1)
}

/// Start one engine per *enabled* account.
///
/// ADR 0005 Q3: the first account is not special, so there is no
/// `first_account` here — any code that treats one differently fails exactly
/// once, in the field. Every enabled account gets an engine of its own, with
/// its own connection, its own queue and its own backoff, so one unreachable
/// server cannot stall the others.
///
/// Refuses rather than truncating when there are more accounts than the pool
/// can serve. Starting nine of ten engines would leave the tenth account
/// looking permanently offline with nothing in the interface explaining why.
#[allow(clippy::too_many_arguments)]
pub fn start_all(
    accounts: &[Account],
    database: &Database,
    blobs: BlobStore,
    events: EventSink,
    secrets: Arc<dyn SecretStore>,
    mailbox_roles: postio_model::RoleOverrides,
    rules: postio_search::rules::RuleSet,
    backfill: postio_runtime::BackfillPolicy,
    watch: postio_sync::WatchPolicy,
    egress: &Arc<crate::egress::EgressRecorder>,
) -> Result<Vec<(AccountId, Engine)>, StartupRefusal> {
    let enabled: Vec<&Account> = accounts.iter().filter(|account| account.enabled).collect();
    let budget = engine_budget(database.pool().max_connections());

    if enabled.len() > budget {
        return Err(StartupRefusal::TooManyAccounts {
            accounts: enabled.len(),
            budget,
        });
    }

    let mut engines = Vec::with_capacity(enabled.len());
    for account in enabled {
        // A transport that cannot be built costs *that* account its sync and
        // nothing else — `start` already logs why. The others still run.
        if let Some(engine) = start(
            account,
            database,
            blobs.clone(),
            events.clone(),
            Arc::clone(&secrets),
            mailbox_roles.clone(),
            rules.clone(),
            backfill,
            watch,
            egress.for_account(account.id),
        ) {
            engines.push((account.id, engine));
        }
    }
    Ok(engines)
}

/// Start the engine for an account joining an application that is already
/// running (#64, ADR 0012 Q2).
///
/// The same work [`start_all`] does per account, with the one thing that
/// genuinely differs when an account arrives on its own: the budget is
/// asked about *the set this account is joining*, not about a set being
/// started from nothing. `accounts` is how many enabled accounts there will
/// be once this one is among them — which is what the caller has just
/// written to the store, and what the pool will have to serve.
///
/// Refuses rather than starting an engine the pool cannot carry, for
/// [`start_all`]'s own reason: an account whose engine never started is an
/// account whose mail silently stops arriving, and the caller is on a
/// surface where it can say so.
///
/// `Ok(None)` is the same "no usable transport" answer [`start`] gives, and
/// costs that account its sync and nothing else.
#[allow(clippy::too_many_arguments)]
pub fn start_joining(
    account: &Account,
    accounts: usize,
    database: &Database,
    blobs: BlobStore,
    events: EventSink,
    secrets: Arc<dyn SecretStore>,
    mailbox_roles: postio_model::RoleOverrides,
    rules: postio_search::rules::RuleSet,
    backfill: postio_runtime::BackfillPolicy,
    watch: postio_sync::WatchPolicy,
    egress: &Arc<crate::egress::EgressRecorder>,
) -> Result<Option<Engine>, StartupRefusal> {
    let budget = engine_budget(database.pool().max_connections());
    if accounts > budget {
        return Err(StartupRefusal::TooManyAccounts { accounts, budget });
    }
    Ok(start(
        account,
        database,
        blobs,
        events,
        secrets,
        mailbox_roles,
        rules,
        backfill,
        watch,
        egress.for_account(account.id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_follows_the_accounts_stored_choice() {
        // #545, ADR 0018 Q5: the adapter is the account's data, chosen at
        // add time from the preset row's preference. One code path above
        // the seam — this match is the whole of it.
        let secrets: Arc<dyn postio_account::secret::SecretStore> =
            Arc::new(postio_account::secret::MemorySecretStore::new());
        let mut account = postio_model::Account::new(
            "Ada",
            postio_model::EmailAddress::new(None::<String>, "ada@example.com"),
        );
        let key = AccountKey::new(account.address.address.clone());
        let tokens = token_source(&account, &secrets);
        let connector = Arc::new(RustlsConnector::new().expect("a connector"));

        let chosen = backend_for(&account, key.clone(), tokens.clone(), connector.clone());
        assert!(
            format!("{chosen:?}").contains("ImapBackend"),
            "the default is the adapter every existing account uses: {chosen:?}"
        );

        account.backend = postio_model::account::Backend::Jmap {
            session_url: "https://api.example.com/jmap/session/".to_string(),
        };
        let chosen = backend_for(&account, key.clone(), tokens.clone(), connector.clone());
        assert!(
            format!("{chosen:?}").contains("JmapBackend"),
            "a stored jmap choice gets the JMAP adapter: {chosen:?}"
        );

        account.backend = postio_model::account::Backend::Gmail;
        let chosen = backend_for(&account, key.clone(), tokens.clone(), connector.clone());
        assert!(
            format!("{chosen:?}").contains("GmailBackend"),
            "a stored gmail choice gets the Gmail REST adapter: {chosen:?}"
        );

        account.backend = postio_model::account::Backend::Jmap {
            session_url: "not a url at all".to_string(),
        };
        let chosen = backend_for(&account, key, tokens, connector);
        assert!(
            format!("{chosen:?}").contains("ImapBackend"),
            "a jmap row whose URL no longer parses falls back to the IMAP \
             adapter rather than an account that can never sync: {chosen:?}"
        );
    }

    #[test]
    fn the_token_source_follows_the_accounts_oauth_shape() {
        // #534, ADR 0006 Q1: which strategy obtains the credential is the
        // account's data. A password account reads the keyring as it
        // always has; an OAuth account that signed in with its own client
        // refreshes through that client; an OAuth account with *no* client
        // of its own is broker-fed — its token is already in the keyring,
        // which is exactly the stored-password shape (#533's path).
        let secrets: Arc<dyn postio_account::secret::SecretStore> =
            Arc::new(postio_account::secret::MemorySecretStore::new());
        let mut account = postio_model::Account::new(
            "Ada",
            postio_model::EmailAddress::new(None::<String>, "ada@example.com"),
        );

        let source = token_source(&account, &secrets);
        assert!(
            format!("{source:?}").contains("StoredPasswordSource"),
            "password accounts keep the keyring path: {source:?}"
        );

        account.auth = postio_model::account::AuthMethod::XOAuth2;
        let source = token_source(&account, &secrets);
        assert!(
            format!("{source:?}").contains("StoredPasswordSource"),
            "broker-fed OAuth carries no client and stays keyring-shaped: {source:?}"
        );

        account.oauth = Some(postio_model::account::OAuthConfig {
            client_id: "postio-desktop.apps.example".to_string(),
            token_url: "https://auth.example.com/token".to_string(),
            authorize_url: "https://auth.example.com/authorize".to_string(),
            scopes: "https://mail.example.com/".to_string(),
        });
        let source = token_source(&account, &secrets);
        assert!(
            format!("{source:?}").contains("OwnClientTokenSource"),
            "an own-client sign-in refreshes through its client: {source:?}"
        );
    }

    #[test]
    fn the_connection_settings_carry_the_accounts_auth_method() {
        // #533: this mapping dropped `auth` on the floor, so an account
        // stored as XOAUTH2 authenticated with LOGIN — the mechanisms #193
        // wired into the sessions were reachable from tests only.
        let server = postio_model::account::ServerConfig {
            host: "imap.example.com".to_owned(),
            port: 993,
            username: "ada@example.com".to_owned(),
            ..Default::default()
        };

        let settings = settings(&server, postio_model::account::AuthMethod::XOAuth2);
        assert_eq!(
            settings.auth,
            postio_model::account::AuthMethod::XOAuth2,
            "the account's auth method must reach the session, or the              stored mechanism is decoration"
        );
    }

    #[test]
    fn a_pool_keeps_one_connection_back_for_the_frontend() {
        // The UI reads on every repaint; an engine that took the last
        // connection would make the window wait on the network it is not
        // allowed to wait on.
        assert_eq!(engine_budget(4), 3);
        assert_eq!(engine_budget(2), 1);
    }

    #[test]
    fn a_single_connection_pool_carries_no_engines_rather_than_deadlocking() {
        assert_eq!(engine_budget(1), 0);
        assert_eq!(engine_budget(0), 0);
    }

    #[test]
    fn the_refusal_says_what_to_do_about_it() {
        let refusal = StartupRefusal::TooManyAccounts {
            accounts: 10,
            budget: 3,
        };
        let said = refusal.to_string();

        assert!(said.contains("10") && said.contains('3'), "{said}");
        assert!(
            said.contains("Disable an account") && said.contains("restart"),
            "a refusal the user cannot act on is a hang with extra steps: {said}"
        );
    }
}
