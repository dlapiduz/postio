//! "Test connection": does this account's *stored* configuration work?
//!
//! # Why not the discovery probe
//!
//! `postio_account::discovery::Probe::run` takes an email address and guesses
//! — presets, common host names, candidate ports. It answers *"what settings
//! should this account use?"*, which is onboarding's question. The button on
//! an existing account's detail view asks the opposite one: *"do the settings
//! this account already has work?"* Running discovery there would probe hosts
//! the user has not configured and could report success for a server they are
//! not using, on the one screen whose whole subject is the stored values
//! (#980).
//!
//! # It needs no new protocol code
//!
//! `ImapSession::open` is documented as "connect, secure, authenticate, read
//! capabilities" and `SmtpSession::open` is its counterpart. Each already *is*
//! a connection test, so this is composition: the stored `ServerConfig`, the
//! credential the account's own auth strategy yields, one attempt per
//! protocol.
//!
//! Through `auth::with_credential`, which is the shared invalidate-and-retry
//! discipline both live paths use (ADR 0006 Q5) — so a token that has gone
//! stale in the keyring is refreshed once here exactly as it would be by a
//! real sync, and the test reports what the *next sync* would find rather
//! than a stale-cache artefact.
//!
//! # Reported per protocol
//!
//! Incoming and outgoing are separate answers because they are separate
//! servers — different hosts, different ports, sometimes different
//! credentials. "It does not work" is not actionable when half of it does,
//! and the half that does is the half the user should stop editing.
//!
//! # Nothing here dials out on its own
//!
//! It runs when somebody presses a button, which is `ARCHITECTURE.md` §11's
//! test for anything that leaves the machine. The connectors are arguments
//! rather than constructed here, so the tests in the default suite reach no
//! network at all — the same reason `postio_app::onboarding::probe` takes its
//! transport (#282).

use std::sync::Arc;

use postio_account::auth::{self, TokenSource};
use postio_account::imap::{ImapConnector, ImapSession};
use postio_account::secret::{AccountKey, SecretStore};
use postio_model::Account;
use postio_smtp::session::SmtpSession;
use postio_smtp::settings::ConnectionSettings as SmtpSettings;
use postio_smtp::transport::SmtpConnector;
use secrecy::SecretString;

/// Whether one server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reachability {
    /// Connected, secured and authenticated.
    Reached,
    /// It did not, and this is what the server or the transport said.
    ///
    /// The server's own words, carried rather than flattened to "could not
    /// connect": a bounce that says *"authentication failed"* and one that
    /// says *"connection refused"* send a person to two different fields, and
    /// the acceptance for #980 is a real error message rather than a spinner
    /// that stops.
    Refused {
        /// What went wrong, phrased for the person reading the settings.
        reason: String,
    },
}

impl Reachability {
    /// Whether the server answered.
    pub fn reached(&self) -> bool {
        matches!(self, Reachability::Reached)
    }
}

/// What a test of one account's stored settings found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reachabilities {
    /// The incoming (IMAP) server.
    pub incoming: Reachability,
    /// The outgoing (SMTP) server.
    pub outgoing: Reachability,
}

impl Reachabilities {
    /// Whether both servers answered.
    pub fn all_reached(&self) -> bool {
        self.incoming.reached() && self.outgoing.reached()
    }
}

/// Try `account`'s stored incoming and outgoing settings, and say what
/// happened to each.
///
/// Both are attempted whatever the first one does: the point of the button is
/// to find out which half needs fixing, and stopping at the first failure
/// would hide a second one behind it for another round trip.
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
    match attempt {
        // The credential itself could not be had -- no keyring entry, a
        // refresh that failed. Redacted, because this string is going on
        // screen *and* into a log: `SecretError` names the account.
        Err(error) => refused(postio_model::address::redact_addresses(&error.to_string())),
        Ok(Err(error)) => refused(error.to_string()),
        Ok(Ok(_session)) => Reachability::Reached,
    }
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
                // One `to_owned`, for the reason `postio-sync`'s send path
                // gives: `SecretString::from` goes through
                // `String::into_boxed_str`, which reallocates when capacity
                // exceeds length and frees the buffer holding the password
                // without overwriting it (#144).
                let password = SecretString::from(credential.expose().to_owned());
                SmtpSession::open(&settings, &password, connector).await
            }
        },
    )
    .await;
    match attempt {
        Err(error) => refused(postio_model::address::redact_addresses(&error.to_string())),
        Ok(Err(error)) => refused(error.to_string()),
        Ok(Ok(_session)) => Reachability::Reached,
    }
}

fn refused(reason: String) -> Reachability {
    Reachability::Refused { reason }
}
