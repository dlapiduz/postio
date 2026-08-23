//! What can go wrong reaching a mail server, expressed without naming one.

use std::time::Duration;

use postio_model::UidValidity;

use crate::secret::SecretError;

use super::Capability;

/// The result of every [`MailBackend`](super::MailBackend) method.
pub type BackendResult<T> = Result<T, BackendError>;

/// Everything a backend can fail with.
///
/// Deliberately protocol-neutral: an IMAP `NO`, an SMTP 5xx and a mock's
/// injected fault all land in the same variants, because the caller's decision
/// — retry, re-authenticate, resync from scratch, or tell the user — does not
/// depend on which protocol produced it.
///
/// Three predicates carry that decision: [`is_transient`](Self::is_transient),
/// [`is_authentication_failure`](Self::is_authentication_failure) and
/// [`requires_full_resync`](Self::requires_full_resync). The sync engine
/// branches on those, never on the variant, so a new variant cannot silently
/// change how existing code retries.
///
/// No variant carries a password, so these are safe to log verbatim.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BackendError {
    /// A command was issued before the backend had a session.
    #[error("not connected: {context}")]
    NotConnected {
        /// What was being attempted.
        context: String,
    },

    /// The session went away underneath a command.
    #[error("the connection was lost during {context}: {reason}")]
    Disconnected {
        /// What was being attempted.
        context: String,
        /// What the transport reported.
        reason: String,
    },

    /// The server did not answer in time.
    #[error("{context} timed out after {}s", after.as_secs_f32())]
    TimedOut {
        /// What was being attempted.
        context: String,
        /// How long we waited.
        after: Duration,
    },

    /// The server rejected the credentials. Retrying will not help.
    #[error("the server rejected the credentials for {account}: {reason}")]
    Auth {
        /// The account that failed to log in.
        account: String,
        /// What the server said.
        reason: String,
    },

    /// The TLS handshake failed, or the certificate did not verify.
    ///
    /// Never recoverable by falling back to plaintext — that decision is not
    /// Postio's to make on the user's behalf.
    #[error("TLS failed for {host}: {reason}")]
    Tls {
        /// The host we were connecting to.
        host: String,
        /// What the TLS stack reported.
        reason: String,
    },

    /// The post-authentication capability list was empty.
    ///
    /// A real server always advertises something. An empty list means the
    /// capability round trip was skipped, and treating that as "supports
    /// nothing" would silently disable QRESYNC, IDLE and UIDPLUS forever.
    /// See ADR 0001, "Binding rules for `postio-imap`", rule 1.
    #[error("{host} reported no capabilities after authentication; refusing to guess")]
    EmptyCapabilities {
        /// The host that said nothing.
        host: String,
    },

    /// The server does not support an extension this operation needs.
    #[error("the server does not support {}", capability.as_str())]
    Unsupported {
        /// The extension that is missing.
        capability: Capability,
    },

    /// No such mailbox on the server.
    #[error("no mailbox named {path}")]
    NoSuchMailbox {
        /// The path that was asked for.
        path: String,
    },

    /// No message with that UID in that mailbox.
    #[error("no message with UID {uid} in {mailbox}")]
    NoSuchMessage {
        /// The mailbox that was searched.
        mailbox: String,
        /// The UID that was asked for.
        uid: u32,
    },

    /// The mailbox's UID space was renumbered; every cached UID is stale.
    #[error(
        "UIDVALIDITY for {mailbox} changed from {known} to {observed}; \
         every cached UID for it is meaningless and the mailbox must be resynchronized"
    )]
    UidValidityChanged {
        /// The affected mailbox.
        mailbox: String,
        /// The generation Postio believed it was working in.
        known: UidValidity,
        /// The generation the server reports now.
        observed: UidValidity,
    },

    /// The server understood the command and refused it.
    #[error("the server refused {command}: {reason}")]
    Rejected {
        /// The command, as a short label — never the arguments, which may
        /// carry message content.
        command: String,
        /// What the server said.
        reason: String,
    },

    /// The server is asking us to slow down.
    #[error("the server is rate limiting us{}: {reason}", retry_after
        .map(|after| format!(" for {}s", after.as_secs()))
        .unwrap_or_default())]
    RateLimited {
        /// How long the server asked us to wait, when it said.
        retry_after: Option<Duration>,
        /// What the server said.
        reason: String,
    },

    /// The server said something we could not make sense of.
    #[error("the server sent a response Postio could not read: {reason}")]
    Protocol {
        /// What was wrong with it.
        reason: String,
    },

    /// The transport failed.
    #[error("{context} failed: {reason}")]
    Io {
        /// What was being attempted.
        context: String,
        /// The underlying error.
        reason: String,
    },

    /// Reaching the account's password failed.
    #[error(transparent)]
    Secret(#[from] SecretError),

    /// The caller cancelled the operation.
    ///
    /// Not a failure — the UI moved on — but it is not a result either, so it
    /// must not be mistaken for an empty answer.
    #[error("cancelled")]
    Cancelled,
}

impl BackendError {
    /// Whether trying the same thing again later could succeed.
    ///
    /// This is the retry predicate the operation queue and the backoff loop
    /// branch on. Authentication, missing capabilities and a renumbered UID
    /// space are all *stable* facts about the server: retrying them just burns
    /// the user's battery.
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

    /// Whether the user has to supply a new password before anything will work.
    pub fn is_authentication_failure(&self) -> bool {
        matches!(self, Self::Auth { .. })
    }

    /// Whether the mailbox's cached state must be thrown away and refetched.
    pub fn requires_full_resync(&self) -> bool {
        matches!(self, Self::UidValidityChanged { .. })
    }

    /// How long the server asked us to wait, when it said so.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}
