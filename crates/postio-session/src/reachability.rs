//! "Test connection": does this account's stored configuration work (#980)? Not the discovery
//! probe, which guesses settings and could report success for a server the user is not using.
//! Composes `ImapSession::open` and `SmtpSession::open` through `auth::with_credential`, so a stale
//! token is refreshed as a real sync would. Incoming and outgoing are reported separately. The
//! connectors are arguments, so the default suite reaches no network.

use std::sync::Arc;

use postio_account::auth::{self, TokenSource};
use postio_account::imap::{ImapConnector, ImapSession};
use postio_account::secret::{AccountKey, SecretError, SecretStore};
use postio_model::Account;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::ConnectionSettings as SmtpSettings;
use postio_smtp::transport::SmtpConnector;
use secrecy::SecretString;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    Reached,
    /// It did not; carries the server's own words, since "authentication failed" and "connection
    /// refused" send a person to different fields.
    Refused {
        reason: String,
        trouble: Option<postio_ui::account::Trouble>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reachabilities {
    pub incoming: Reachability,
    pub outgoing: Reachability,
}

impl Reachabilities {
    pub fn into_results(self) -> (Result<(), String>, Result<(), String>) {
        let result = |reachability| match reachability {
            Reachability::Reached => Ok(()),
            Reachability::Refused { reason, .. } => Err(reason),
        };
        (result(self.incoming), result(self.outgoing))
    }

    pub fn trouble(&self) -> Option<postio_ui::account::Trouble> {
        [&self.incoming, &self.outgoing]
            .into_iter()
            .find_map(|reachability| match reachability {
                Reachability::Refused { trouble, .. } => *trouble,
                Reachability::Reached => None,
            })
    }
}

/// `test_connection` over the real TLS connectors. A connector that will not build is a TLS stack
/// problem and both halves say so.
pub async fn test_over_tls(account: &Account, secrets: &Arc<dyn SecretStore>) -> Reachabilities {
    match (
        postio_account::imap::RustlsConnector::new(),
        postio_smtp::transport::RustlsConnector::new(),
    ) {
        (Ok(imap), Ok(smtp)) => test_connection(account, secrets, &imap, &smtp).await,
        (imap, smtp) => {
            let reason = imap
                .err()
                .map(|error| error.to_string())
                .or_else(|| smtp.err().map(|error| error.to_string()))
                .unwrap_or_else(|| "the TLS stack would not start".to_owned());
            Reachabilities {
                incoming: refused(reason.clone()),
                outgoing: refused(reason),
            }
        }
    }
}

/// When each of `accounts` has its stored sign-in token expire: `None` where nothing was persisted
/// (password accounts, broker-signed accounts) (#878).
pub async fn token_expiries(
    secrets: &dyn SecretStore,
    accounts: Vec<(postio_model::ids::AccountId, String)>,
) -> Vec<(postio_model::ids::AccountId, Option<std::time::SystemTime>)> {
    let mut expiries = Vec::with_capacity(accounts.len());
    for (id, address) in accounts {
        let key = AccountKey::new(address);
        let expiry = postio_account::oauth::token_source::stored_expiry(secrets, &key).await;
        expiries.push((id, expiry));
    }
    expiries
}

/// Try `account`'s stored incoming and outgoing settings and say what happened to each; both are
/// attempted so a second failure is not hidden behind the first.
pub async fn test_connection(
    account: &Account,
    secrets: &Arc<dyn SecretStore>,
    imap: &dyn ImapConnector,
    smtp: &dyn SmtpConnector,
) -> Reachabilities {
    let key = AccountKey::new(account.address.address.clone());
    let tokens = crate::engine::token_source(account, secrets);

    Reachabilities {
        incoming: try_incoming(account, &key, tokens.as_ref(), imap).await,
        outgoing: try_outgoing(account, &key, tokens.as_ref(), smtp).await,
    }
}

async fn try_incoming(
    account: &Account,
    key: &AccountKey,
    tokens: &dyn TokenSource,
    connector: &dyn ImapConnector,
) -> Reachability {
    let settings = crate::engine::settings(&account.incoming, account.auth);
    let attempt = auth::with_credential(
        tokens,
        key,
        postio_account::backend::BackendError::is_authentication_failure,
        |credential| {
            let settings = settings.clone();
            async move { ImapSession::open(&settings, &credential, connector).await }
        },
    )
    .await;
    judged(
        attempt.map(|opened| opened.map(drop)),
        postio_account::backend::BackendError::is_authentication_failure,
    )
}

async fn try_outgoing(
    account: &Account,
    key: &AccountKey,
    tokens: &dyn TokenSource,
    connector: &dyn SmtpConnector,
) -> Reachability {
    let settings = SmtpSettings::from_server_config(&account.outgoing).with_auth(account.auth);
    let attempt = auth::with_credential(
        tokens,
        key,
        postio_smtp::error::SmtpError::is_authentication_failure,
        |credential| {
            let settings = settings.clone();
            async move {
                // One `to_owned`: `SecretString::from` reallocates and frees the password buffer
                // without overwriting it (#144).
                let password = SecretString::from(credential.expose().to_owned());
                SmtpSession::open(&settings, &password, connector).await
            }
        },
    )
    .await;
    judged(
        attempt.map(|opened| opened.map(drop)),
        postio_smtp::error::SmtpError::is_authentication_failure,
    )
}

fn judged<E: std::fmt::Display>(
    attempt: Result<Result<(), E>, SecretError>,
    rejected: impl Fn(&E) -> bool,
) -> Reachability {
    use postio_ui::account::Trouble;
    match attempt {
        Ok(Ok(())) => Reachability::Reached,
        // Redacted: this string goes on screen and into a log, and `SecretError` names the account.
        Err(error) => Reachability::Refused {
            reason: postio_model::address::redact_addresses(&error.to_string()),
            trouble: match error {
                SecretError::NotFound { .. } => Some(Trouble::Missing),
                SecretError::GrantExpired { .. } => Some(Trouble::Expired),
                _ => None,
            },
        },
        Ok(Err(error)) => Reachability::Refused {
            reason: error.to_string(),
            trouble: rejected(&error).then_some(Trouble::Rejected),
        },
    }
}

fn refused(reason: String) -> Reachability {
    Reachability::Refused {
        reason,
        trouble: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_ui::account::Trouble;

    #[derive(Debug)]
    struct Said(&'static str, bool);

    impl std::fmt::Display for Said {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    fn trouble(attempt: Result<Result<(), Said>, SecretError>) -> Option<Trouble> {
        match judged(attempt, |said| said.1) {
            Reachability::Refused { trouble, .. } => trouble,
            Reachability::Reached => None,
        }
    }

    #[test]
    fn a_password_the_server_refused_is_credential_trouble() {
        assert_eq!(
            trouble(Ok(Err(Said("authentication failed", true)))),
            Some(Trouble::Rejected)
        );
    }

    #[test]
    fn nothing_in_the_keyring_and_an_expired_grant_are_credential_trouble() {
        let account = "ada@example.com".to_owned();
        assert_eq!(
            trouble(Err(SecretError::NotFound {
                account: account.clone()
            })),
            Some(Trouble::Missing)
        );
        assert_eq!(
            trouble(Err(SecretError::GrantExpired { account })),
            Some(Trouble::Expired)
        );
    }

    #[test]
    fn a_server_that_did_not_answer_is_no_credential_trouble() {
        assert_eq!(trouble(Ok(Err(Said("connection refused", false)))), None);
        assert_eq!(judged::<Said>(Ok(Ok(())), |_| true), Reachability::Reached);
    }
}
