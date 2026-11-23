//! Building the sync engine for an account: choices about this installation (TLS stack, keyring,
//! pool size), taken as parts by `postio-runtime` so tests can substitute them.
//! Nothing here dials: `ImapBackend::new` opens no connection, so Postio starts and says so with no
//! network.
//! `NetworkSource::NetworkManager` is asked for here, not defaulted in the engine, so tests behave
//! the same on a desktop and in CI; an engine over `Wiring::with_mail` does not ask.

use std::sync::Arc;

use crate::Wiring;
use postio_account::auth::{StoredPasswordSource, TokenSource};
use postio_account::backend::MailBackend;
use postio_account::imap::{
    ConnectionPool, ConnectionSettings, ImapBackend, PoolConfig, RustlsConnector,
};
use postio_account::secret::{AccountKey, SecretStore};
use postio_model::{Account, AccountId};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};

/// Start the engine for `account`, from this installation's `Wiring`. `None` when the transports
/// cannot be built (no usable TLS stack); the local store still opens.
pub fn start(account: &Account, wiring: &Wiring) -> Option<Engine> {
    let key = AccountKey::new(account.address.address.clone());

    let egress = wiring.egress.for_account(account.id);
    let connector = match RustlsConnector::new() {
        Ok(connector) => Arc::new(
            connector.with_egress(egress.clone(), postio_model::egress::EgressSubsystem::Imap),
        ),
        Err(error) => {
            tracing::error!(%error, "no IMAP transport, so no sync: {error}");
            return None;
        }
    };
    let smtp = match postio_smtp::transport::RustlsConnector::new() {
        Ok(connector) => {
            Arc::new(connector.with_egress(egress, postio_model::egress::EgressSubsystem::Smtp))
        }
        Err(error) => {
            tracing::error!(%error, "no SMTP transport, so nothing can be sent: {error}");
            return None;
        }
    };

    // One `TokenSource` per account, shared by both sides (ADR 0006 Q5): a second one for SMTP
    // would not see a rejection seen while fetching, and two refreshes on a rotating-refresh-token
    // provider invalidate each other.
    let tokens = token_source(account, &wiring.secrets);

    let (backend, smtp) = match &wiring.mail {
        Some(mail) => (mail.backend.clone(), mail.smtp.clone()),
        None => {
            let smtp: Arc<dyn postio_smtp::transport::SmtpConnector> = smtp;
            (backend_for(account, key, tokens.clone(), connector), smtp)
        }
    };

    match Engine::spawn_filing(
        EngineParts {
            account: account.id,
            database: wiring.database.clone(),
            blobs: wiring.blobs.clone(),
            backend,
            smtp,
            tokens,
            events: wiring.events.clone(),
            retry: Default::default(),
            backfill: wiring.backfill,
            reconnect: Default::default(),
            watch: wiring.watch,
            network: network_for(wiring.mail.as_ref()),
            mailbox_roles: wiring.mailbox_roles.clone(),
            clock: Arc::new(SystemClock),
        },
        wiring.filing.clone(),
    ) {
        Ok(engine) => Some(engine),
        Err(error) => {
            tracing::error!(%error, "the sync engine did not start: {error}");
            None
        }
    }
}

/// Who tells an engine about the network: NetworkManager, unless the mail goes somewhere else. A
/// mock server is not on the machine's network, so following the desktop would make host tests
/// depend on it.
fn network_for(mail: Option<&crate::MailOverride>) -> NetworkSource {
    match mail {
        Some(_) => NetworkSource::Ignored,
        None => NetworkSource::NetworkManager,
    }
}

/// The adapter the account's stored backend choice names (#545, ADR 0018 Q5). A `jmap` row whose
/// session URL no longer parses falls back to IMAP with an error logged.
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
                    "the stored JMAP session URL does not parse; falling back to IMAP: {error}"
                );
            }
        },
        postio_model::account::Backend::Gmail => {
            return Arc::new(postio_gmail::GmailBackend::with_token_source(key, tokens));
        }
        postio_model::account::Backend::Maildir { root } => {
            // Unchecked: a maildir account has no incoming server to fall back to, so a moved tree
            // fails at connect, not at build.
            return Arc::new(postio_account::maildir::MaildirBackend::at(root));
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

/// Which strategy obtains this account's credential (ADR 0006 Q1, #534): an `OAuthConfig` on the
/// row means refresh through the user's own client; none means the keyring entry is the credential
/// (password, app password or broker-fed token).
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
                        // Copied onto the row at sign-in so a launch needs neither the preset table
                        // nor a network (#954).
                        oauth
                            .refresh_token_lifetime_days
                            .map(|days| std::time::Duration::from_secs(u64::from(days) * 86_400)),
                    ),
                );
            }
            Err(error) => {
                // A row this build cannot parse must not cost the account its mail; the keyring may
                // hold a workable token.
                tracing::error!(
                    %error,
                    "the account's stored OAuth token endpoint is not a URL; \
                     falling back to the stored credential: {error}"
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
    // Carry the stored mechanism through; dropping it authenticated every account as Password
    // (#533).
    settings.auth = auth;
    settings
}

/// A refusal rather than a silent truncation: an account whose engine never started silently stops
/// receiving mail (#183).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupRefusal {
    TooManyAccounts { accounts: usize, budget: usize },
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

/// How many engines a pool of `max_connections` can carry: one per engine, one left for the
/// frontend, which reads on every repaint and must not wait.
pub fn engine_budget(max_connections: usize) -> usize {
    max_connections.saturating_sub(1)
}

/// Start one engine per enabled account (ADR 0005 Q3: the first account is not special). Refuses
/// rather than truncating when there are more accounts than the pool can serve.
pub async fn start_all(
    accounts: &[Account],
    wiring: &Wiring,
) -> Result<Vec<(AccountId, Engine)>, StartupRefusal> {
    let enabled: Vec<&Account> = accounts.iter().filter(|account| account.enabled).collect();
    let budget = engine_budget(postio_storage::MAX_CONCURRENT_PASSES);

    if enabled.len() > budget {
        return Err(StartupRefusal::TooManyAccounts {
            accounts: enabled.len(),
            budget,
        });
    }

    let mut engines = Vec::with_capacity(enabled.len());
    for account in enabled {
        if let Some(engine) = start(account, wiring) {
            engines.push((account.id, engine));
        }
    }
    Ok(engines)
}

/// Start the engine for an account joining a running application (#64, ADR 0012 Q2). `accounts`
/// counts the enabled accounts once this one is among them; refuses as `start_all` does. `Ok(None)`
/// means no usable transport.
pub async fn start_joining(
    account: &Account,
    accounts: usize,
    wiring: &Wiring,
) -> Result<Option<Engine>, StartupRefusal> {
    let budget = engine_budget(postio_storage::MAX_CONCURRENT_PASSES);
    if accounts > budget {
        return Err(StartupRefusal::TooManyAccounts { accounts, budget });
    }
    Ok(start(account, wiring))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_engine_over_a_substitute_server_does_not_follow_the_desktops_network() {
        let mail = crate::MailOverride {
            backend: Arc::new(postio_account::backend::MockBackend::new()),
            smtp: Arc::new(postio_smtp::transport::ScriptedConnector::new(
                postio_smtp::transport::SmtpScript::new("220 ready"),
            )),
        };
        assert_eq!(network_for(Some(&mail)), NetworkSource::Ignored);
        assert_eq!(network_for(None), NetworkSource::NetworkManager);
    }

    #[test]
    fn the_backend_follows_the_accounts_stored_choice() {
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
            refresh_token_lifetime_days: None,
        });
        let source = token_source(&account, &secrets);
        assert!(
            format!("{source:?}").contains("OwnClientTokenSource"),
            "an own-client sign-in refreshes through its client: {source:?}"
        );
    }

    #[test]
    fn the_connection_settings_carry_the_accounts_auth_method() {
        // #533: this mapping dropped `auth`, so an XOAUTH2 account authenticated with LOGIN.
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
