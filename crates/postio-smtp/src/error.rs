//! What can go wrong sending mail, expressed without naming a provider.
//!
//! Mirrors `postio_imap::backend::BackendError`'s shape: callers branch on
//! [`SmtpError::is_transient`] and [`is_authentication_failure`
//! ](SmtpError::is_authentication_failure), never on the variant, so a new
//! variant cannot silently change how existing code retries. No variant
//! carries a password, so these are safe to log verbatim.

use std::time::Duration;

/// The result of every operation in this crate.
pub type SmtpResult<T> = Result<T, SmtpError>;

/// Everything sending a message can fail with.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SmtpError {
    /// The connection settings are not usable as given — an empty host, or
    /// a mailbox address malformed enough that sending it to a server would
    /// be unsafe.
    #[error("{reason}")]
    Configuration {
        /// What was wrong.
        reason: String,
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

    /// The server rejected the credentials with a permanent (5xx) reply.
    /// Retrying will not help until the user supplies a new password.
    #[error("the server rejected the credentials for {account}: {code} {reason}")]
    Auth {
        /// The account that failed to authenticate.
        account: String,
        /// The SMTP reply code.
        code: u16,
        /// What the server said.
        reason: String,
    },

    /// The server rejected the sender address with a permanent (5xx) reply
    /// to `MAIL FROM`.
    #[error("the server rejected the sender {sender}: {code} {reason}")]
    SenderRejected {
        /// The reverse-path address that was refused.
        sender: String,
        /// The SMTP reply code.
        code: u16,
        /// What the server said.
        reason: String,
    },

    /// The server rejected a recipient address with a permanent (5xx) reply
    /// to `RCPT TO`.
    #[error("the server rejected the recipient {recipient}: {code} {reason}")]
    RecipientRejected {
        /// The forward-path address that was refused.
        recipient: String,
        /// The SMTP reply code.
        code: u16,
        /// What the server said.
        reason: String,
    },

    /// The server rejected the message body with a permanent (5xx) reply
    /// to `DATA`, or to the terminating `.`.
    #[error("the server rejected the message: {code} {reason}")]
    MessageRejected {
        /// The SMTP reply code.
        code: u16,
        /// What the server said.
        reason: String,
    },

    /// A transient negative (4xx) reply — the server is asking to try
    /// again later. Covers rate limiting and temporary mailbox or resource
    /// unavailability alike; SMTP has no separate signal for the two.
    #[error("the server asked to try again later: {code} {reason}")]
    Transient {
        /// The SMTP reply code.
        code: u16,
        /// What the server said.
        reason: String,
    },

    /// The server said something we could not make sense of.
    #[error("the server sent a reply Postio could not read: {reason}")]
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

    /// The caller cancelled the operation.
    ///
    /// Not a failure — the user moved on — but it is not a result either,
    /// so it must not be mistaken for a successful send.
    #[error("cancelled")]
    Cancelled,
}

impl SmtpError {
    /// Whether trying the same thing again later could succeed.
    ///
    /// This is the retry predicate a backoff loop branches on. A dropped
    /// connection, a timeout and a 4xx reply — rate limiting included — are
    /// all worth retrying; a rejected credential or address is a stable
    /// fact that retrying will not change.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Disconnected { .. }
                | Self::TimedOut { .. }
                | Self::Io { .. }
                | Self::Transient { .. }
        )
    }

    /// Whether the message payload may already have reached the server.
    ///
    /// The question [`is_transient`](Self::is_transient) cannot answer and a
    /// send has to ask first (ADR 0021). Retrying an archive is harmless;
    /// retrying a send that was in fact accepted delivers a second copy to
    /// somebody else's inbox, and nothing can recall it.
    ///
    /// **False means the server answered and did not accept.** A rejected
    /// credential, sender, recipient or message, a configuration problem, a
    /// TLS failure, an unreadable reply, and a `4xx` — which is a *reply*,
    /// so the server was still talking — all mean the transaction ended in a
    /// refusal the client witnessed. Those are safely retryable, and a caller
    /// may undo whatever it did in anticipation of the send.
    ///
    /// **True means the client stopped hearing from the server**, and there
    /// is no way from here to tell a connection lost before `MAIL FROM` from
    /// one lost between the terminating `.` and the reply to it. This is
    /// deliberately the conservative approximation:
    /// [#673](https://github.com/dlapiduz/postio/issues/673) teaches `data`
    /// where the payload boundary is and narrows it to failures raised once
    /// the payload had begun being written. Until then every transport
    /// failure is treated as though it might have been that one, because the
    /// costs are not symmetric.
    pub fn submission_is_indeterminate(&self) -> bool {
        matches!(
            self,
            Self::Disconnected { .. } | Self::TimedOut { .. } | Self::Io { .. } | Self::Cancelled
        )
    }

    /// Whether the user has to supply a new password before anything will
    /// work.
    pub fn is_authentication_failure(&self) -> bool {
        matches!(self, Self::Auth { .. })
    }

    /// The recipient address a permanent rejection named, when this is one.
    pub fn rejected_recipient(&self) -> Option<&str> {
        match self {
            Self::RecipientRejected { recipient, .. } => Some(recipient),
            _ => None,
        }
    }
}
