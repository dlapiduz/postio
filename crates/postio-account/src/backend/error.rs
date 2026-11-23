use std::time::Duration;

use postio_model::UidValidity;

use crate::secret::SecretError;

use super::Capability;

pub type BackendResult<T> = Result<T, BackendError>;

/// Callers branch on `is_transient`, `is_authentication_failure` and `requires_full_resync`, never
/// on the variant.
/// No variant carries a password, so these are safe to log.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BackendError {
    #[error("not connected: {context}")]
    NotConnected { context: String },

    #[error("the connection was lost during {context}: {reason}")]
    Disconnected { context: String, reason: String },

    #[error("{context} timed out after {}s", after.as_secs_f32())]
    TimedOut { context: String, after: Duration },

    /// The server rejected the credentials. Retrying will not help.
    #[error("the server rejected the credentials for {account}: {reason}")]
    Auth { account: String, reason: String },

    /// Never recoverable by falling back to plaintext.
    #[error("TLS failed for {host}: {reason}")]
    Tls { host: String, reason: String },

    /// An empty list means the capability round trip was skipped; reading it as "supports nothing"
    /// would disable QRESYNC, IDLE and UIDPLUS forever (ADR 0001).
    #[error("{host} reported no capabilities after authentication; refusing to guess")]
    EmptyCapabilities { host: String },

    #[error("the server does not support {}", capability.as_str())]
    Unsupported { capability: Capability },

    #[error("no mailbox named {path}")]
    NoSuchMailbox { path: String },

    #[error("no message with UID {uid} in {mailbox}")]
    NoSuchMessage { mailbox: String, uid: u32 },

    /// The mailbox's UID space was renumbered; every cached UID is stale.
    #[error(
        "UIDVALIDITY for {mailbox} changed from {known} to {observed}; \
         every cached UID for it is meaningless and the mailbox must be resynchronized"
    )]
    UidValidityChanged {
        mailbox: String,
        known: UidValidity,
        observed: UidValidity,
    },

    /// `io-imap` drops undecodable untagged responses without failing the command. A `CHANGEDSINCE`
    /// fetch that skipped any cannot be trusted: resync from scratch.
    #[error(
        "{mailbox}: {skipped} untagged response(s) io-imap could not decode were \
         dropped during an incremental fetch; the result cannot be trusted as complete"
    )]
    ResyncIntegrityLost { mailbox: String, skipped: u64 },

    #[error("the server refused {command}: {reason}")]
    Rejected {
        /// The command, as a short label — never the arguments, which may
        /// carry message content.
        command: String,
        reason: String,
    },

    #[error("the server is rate limiting us{}: {reason}", retry_after
        .map(|after| format!(" for {}s", after.as_secs()))
        .unwrap_or_default())]
    RateLimited {
        /// How long the server asked us to wait, when it said.
        retry_after: Option<Duration>,
        reason: String,
    },

    #[error("the server sent a response Postio could not read: {reason}")]
    Protocol { reason: String },

    #[error("{context} failed: {reason}")]
    Io { context: String, reason: String },

    #[error(transparent)]
    Secret(#[from] SecretError),

    /// Not a failure, but not a result either: it must not be mistaken for an empty answer.
    #[error("cancelled")]
    Cancelled,
}

impl BackendError {
    /// Authentication, missing capabilities and a renumbered UID space are stable facts; retrying
    /// them burns battery.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::NotConnected { .. }
                | Self::Disconnected { .. }
                | Self::TimedOut { .. }
                | Self::RateLimited { .. }
                | Self::Io { .. }
        )
    }

    pub fn is_authentication_failure(&self) -> bool {
        matches!(
            self,
            Self::Auth { .. } | Self::Secret(SecretError::GrantExpired { .. })
        )
    }

    /// A refused or expired password, or none in the keyring. Nothing reached the server, so no
    /// attempt counts against the account.
    pub fn needs_a_password(&self) -> bool {
        self.is_authentication_failure()
            || matches!(self, Self::Secret(SecretError::NotFound { .. }))
    }

    pub fn requires_full_resync(&self) -> bool {
        matches!(
            self,
            Self::UidValidityChanged { .. } | Self::ResyncIntegrityLost { .. }
        )
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expired_grant_is_an_authentication_failure() {
        // The connection layer asks only this, so an expired grant must answer like a refusal
        // (#954).
        let expired = BackendError::Secret(SecretError::GrantExpired {
            account: "ada@example.com".to_owned(),
        });
        assert!(expired.is_authentication_failure());
    }

    #[test]
    fn a_locked_keyring_is_not_an_authentication_failure() {
        // A locked keyring is fixed by unlocking it; telling the user to sign in again would redo a
        // good sign-in.
        let locked = BackendError::Secret(SecretError::Locked {
            keyring: "login".to_owned(),
            account: "ada@example.com".to_owned(),
        });
        assert!(!locked.is_authentication_failure());
    }

    #[test]
    fn a_missing_credential_is_not_one_either() {
        let missing = BackendError::Secret(SecretError::NotFound {
            account: "ada@example.com".to_owned(),
        });
        assert!(!missing.is_authentication_failure());
        assert!(missing.needs_a_password(), "but it does need a password");
    }
}
