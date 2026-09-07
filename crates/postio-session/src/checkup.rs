//! Asking an account whether it still works, and taking one away (#1277).
//!
//! Three account actions the settings window draws and neither frontend could
//! run: **test the connection**, **re-index the store**, and **remove the
//! account**. They belong together because they share the two things that
//! make them awkward — reaching a credential that lives in the keyring, and
//! saying what happened in words somebody can act on.
//!
//! # The words are the feature
//!
//! "Test connection" that answers *no* is worthless. `BackendError` already
//! distinguishes the cases that call for different actions — a rejected
//! password, a TLS failure, a timeout, a port that answers but is not IMAP —
//! and turning those into sentences is a rule, not a rendering, so it lives
//! here rather than in each frontend. The wording is `postio-app`'s own,
//! moved: it knows the one thing the error cannot, which is that a provider
//! refusing an ordinary account password says only "rejected".
//!
//! No variant of `BackendError` carries a credential, so these are safe to
//! show and safe to log.

use std::sync::Arc;

use postio_account::backend::BackendError;
use postio_account::imap::{ImapSession, RustlsConnector};
use postio_account::secret::{AccountKey, SecretStore};
use postio_model::Account;
use postio_model::ids::AccountId;
use postio_storage::Database;
use postio_storage::repository::AccountRepository;

/// What a connection test found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    /// Whether the account could sign in.
    pub reachable: bool,
    /// What happened, for the person who pressed the button.
    pub message: String,
    /// Whether the failure was that there is **no credential** for this
    /// account rather than a wrong or rejected one.
    ///
    /// The settings pane's *Partial* state: a row that exists with nothing in
    /// the keyring to sign in with. It calls for a different offer — put a
    /// password in, or sign in again — than a server that said no, and a
    /// pane that could not tell them apart would offer the wrong one.
    pub missing_credential: bool,
}

/// Open a session against the account's own server and close it again.
///
/// What sync does, and then stops. The credential comes from wherever this
/// account keeps one — a stored password, an app password, or a refreshed
/// OAuth token — through the same `TokenSource` the engine uses, so a test
/// that passes is a statement about the path sync will take rather than
/// about a second one written for the button.
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
        },
        Err(error) => ConnectionReport {
            reachable: false,
            message: explain(&error),
            missing_credential: false,
        },
    }
}

/// Take an account away: its row, and then its credential.
///
/// **The row first here, which is the opposite of adding one**, and for the
/// same reason: what must never be left behind is a credential nothing names
/// versus an account that cannot authenticate. Removing leaves the harmless
/// one if it fails halfway — an account gone from the store with a secret
/// still in the keyring is untidy, while a row whose credential had already
/// been deleted would be an account that could not sync and could not be
/// repaired.
///
/// The credential is deleted **on the way out** rather than left: a mail
/// client that forgets an account and keeps its password is the one thing
/// worse than not forgetting it.
pub async fn remove_account(
    database: &Database,
    secrets: Arc<dyn SecretStore>,
    account: AccountId,
) -> Result<(), String> {
    let address = {
        let connection = database
            .connection()
            .map_err(|error| format!("Postio could not open its local store: {error}"))?;
        let repository = AccountRepository::new(&connection);
        let Some(found) = repository
            .get(account)
            .map_err(|error| format!("Postio could not read its local store: {error}"))?
        else {
            // Already gone. Not an error: pressing Remove twice means what it
            // said the first time.
            return Ok(());
        };
        let address = found.address.address.clone();
        repository
            .delete(account)
            .map_err(|error| format!("The account could not be removed: {error}"))?;
        address
    };

    // Best effort, and deliberately after the row: a keyring that refuses
    // must not leave the account half-removed. What it leaves instead is a
    // secret nothing names.
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

/// Change what an account calls itself.
///
/// The one field the canvas' account form edits. Everything else on a row —
/// the servers, the auth method — came from the preset table or from a
/// sign-in, and editing those is changing *which account this is*, which is
/// adding one.
pub fn set_display_name(database: &Database, account: AccountId, name: &str) -> Result<(), String> {
    let connection = database
        .connection()
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    let repository = AccountRepository::new(&connection);
    let Some(mut found) = repository
        .get(account)
        .map_err(|error| format!("Postio could not read its local store: {error}"))?
    else {
        return Err("That account is not in the store.".to_owned());
    };
    found.display_name = name.trim().to_owned();
    repository
        .update(&mut found)
        .map_err(|error| format!("The account could not be updated: {error}"))
}

/// Turn a backend error into something the user can act on.
///
/// Moved from `postio-app::onboarding`, which had it for the same button on
/// the other platform. The interesting case is the first: a provider that
/// refuses ordinary account passwords says only that the credentials were
/// rejected, and somebody who has typed their Apple ID password has no way
/// to tell that from a typo.
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
