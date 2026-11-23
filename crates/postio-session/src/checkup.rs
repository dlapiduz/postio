//! Testing an account's connection, re-indexing, and removing an account (#1277).
//! Turning `BackendError` into sentences someone can act on is a rule, so it lives here; no variant
//! carries a credential, so they are safe to show and log.

use std::sync::Arc;

use postio_account::backend::BackendError;
use postio_account::imap::{ImapSession, RustlsConnector};
use postio_account::secret::{AccountKey, SecretStore};
use postio_model::Account;
use postio_model::ids::AccountId;
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    pub reachable: bool,
    pub message: String,
    /// No credential for this account, rather than a wrong one: the settings pane's *Partial*
    /// state, which calls for a different offer.
    pub missing_credential: bool,
    /// The server refused the credential given (a rotated app password or revoked grant); a refused
    /// credential never passes a second test (#1584).
    pub rejected_credential: bool,
}

/// Open a session against the account's own server and close it again, through the same
/// `TokenSource` sync uses, so a pass says something about sync's path.
pub async fn test_connection(account: &Account, secrets: Arc<dyn SecretStore>) -> ConnectionReport {
    let key = AccountKey::new(account.address.address.clone());
    let tokens = crate::engine::token_source(account, &secrets);
    let credential = match postio_account::auth::TokenSource::access_token(&*tokens, &key).await {
        Ok(credential) => credential,
        Err(error) => {
            return ConnectionReport {
                reachable: false,
                message: format!(
                    "Postio could not read this account's credential from the \
                     keyring: {error}. Is the keyring unlocked?"
                ),
                missing_credential: true,
                rejected_credential: false,
            };
        }
    };

    let settings = crate::engine::settings(&account.incoming, account.auth);
    let connector = match RustlsConnector::new() {
        Ok(connector) => connector,
        Err(error) => {
            return ConnectionReport {
                reachable: false,
                message: format!(
                    "Postio could not start a TLS connection on this machine: {error}"
                ),
                missing_credential: false,
                rejected_credential: false,
            };
        }
    };
    match ImapSession::open(&settings, &credential, &connector).await {
        Ok(_) => ConnectionReport {
            reachable: true,
            message: format!(
                "{} answered and accepted this account.",
                account.incoming.host
            ),
            missing_credential: false,
            rejected_credential: false,
        },
        Err(error) => failure(&error),
    }
}

pub fn failure(error: &BackendError) -> ConnectionReport {
    ConnectionReport {
        reachable: false,
        message: explain(error),
        missing_credential: false,
        rejected_credential: error.is_authentication_failure(),
    }
}

/// Take an account away: its row, then its credential. The row goes first so a halfway failure
/// leaves a harmless orphan secret rather than an account that cannot authenticate or be repaired.
pub async fn remove_account(
    database: &Store,
    secrets: Arc<dyn SecretStore>,
    account: AccountId,
) -> Result<(), String> {
    let address = {
        let connection = database
            .connect()
            .await
            .map_err(|error| format!("Postio could not open its local store: {error}"))?;
        let repository = AccountRepository::new(&connection);
        let Some(found) = repository
            .get(account)
            .await
            .map_err(|error| format!("Postio could not read its local store: {error}"))?
        else {
            return Ok(());
        };
        let address = found.address.address.clone();
        repository
            .delete(account)
            .await
            .map_err(|error| format!("The account could not be removed: {error}"))?;
        address
    };

    // Best effort and after the row: a refusing keyring must not leave the account half-removed.
    for key in [
        AccountKey::new(address.clone()),
        AccountKey::new(format!("{address}#oauth-refresh")),
        AccountKey::new(format!("{address}#oauth-client-secret")),
    ] {
        if let Err(error) = secrets.delete(&key).await {
            tracing::warn!(%error, "a credential for a removed account could not be deleted");
        }
    }
    Ok(())
}

/// Change what an account calls itself, the one field the account form edits; the rest would make
/// it a different account.
pub async fn set_display_name(
    database: &Store,
    account: AccountId,
    name: &str,
) -> Result<(), String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    let repository = AccountRepository::new(&connection);
    let Some(mut found) = repository
        .get(account)
        .await
        .map_err(|error| format!("Postio could not read its local store: {error}"))?
    else {
        return Err("That account is not in the store.".to_owned());
    };
    found.display_name = name.trim().to_owned();
    repository
        .update(&mut found)
        .await
        .map_err(|error| format!("The account could not be updated: {error}"))
}

/// Turn a backend error into something the user can act on. A provider that refuses ordinary
/// account passwords says only "rejected", which a typo looks the same as.
pub fn explain(error: &BackendError) -> String {
    match error {
        BackendError::Auth { .. } => "The server rejected that address and password.\n\n\
             If this is iCloud, Google or another provider with two-factor \
             authentication, your ordinary account password will not work \
             here — you need an app-specific password."
            .to_owned(),
        BackendError::Tls { host, reason } => format!(
            "The secure connection to {host} could not be established: {reason}.\n\n\
             Postio will not fall back to an unencrypted connection. Check the \
             host name and port."
        ),
        BackendError::TimedOut { after, .. } => format!(
            "The server did not answer within {}s. Check the host name and \
             port, and whether this machine can reach the internet.",
            after.as_secs_f32().round()
        ),
        BackendError::Disconnected { reason, .. } => format!(
            "The connection was lost while signing in: {reason}. That usually \
             means the wrong port, or a server that is not IMAP."
        ),
        BackendError::EmptyCapabilities { host } => format!(
            "{host} answered, but not like an IMAP server. Check the host name \
             and port."
        ),
        other => format!("{other}"),
    }
}
