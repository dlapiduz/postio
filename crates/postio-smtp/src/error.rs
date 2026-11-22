//! Callers branch on [`SmtpError::is_transient`] and
//! [`is_authentication_failure`](SmtpError::is_authentication_failure), never on the variant, so a
//! new variant cannot silently change retry behaviour. No variant carries a password, so these are
//! safe to log verbatim.

use std::time::Duration;

pub type SmtpResult<T> = Result<T, SmtpError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SmtpError {
    #[error("{reason}")]
    Configuration { reason: String },

    #[error("the connection was lost during {context}: {reason}")]
    Disconnected { context: String, reason: String },

    #[error("{context} timed out after {}s", after.as_secs_f32())]
    TimedOut { context: String, after: Duration },

    /// Never recoverable by falling back to plaintext: that is not Postio's decision.
    #[error("TLS failed for {host}: {reason}")]
    Tls { host: String, reason: String },

    /// Retrying will not help until the user supplies a new password.
    #[error("the server rejected the credentials for {account}: {code} {reason}")]
    Auth {
        account: String,
        code: u16,
        reason: String,
    },

    #[error("the server rejected the sender {sender}: {code} {reason}")]
    SenderRejected {
        sender: String,
        code: u16,
        reason: String,
    },

    #[error("the server rejected the recipient {recipient}: {code} {reason}")]
    RecipientRejected {
        recipient: String,
        code: u16,
        reason: String,
    },

    #[error("the server rejected the message: {code} {reason}")]
    MessageRejected { code: u16, reason: String },

    /// Covers rate limiting and temporary mailbox unavailability alike; SMTP has no separate
    /// signal.
    #[error("the server asked to try again later: {code} {reason}")]
    Transient { code: u16, reason: String },

    #[error("the server sent a reply Postio could not read: {reason}")]
    Protocol { reason: String },

    #[error("{context} failed: {reason}")]
    Io { context: String, reason: String },

    /// Not a failure, but not a result either: it must not be mistaken for a successful send.
    #[error("cancelled")]
    Cancelled,

    /// How the `DATA` path carries the one fact SMTP cannot report, that the bytes had started
    /// leaving, out to
    /// [`submission_is_indeterminate`](Self::submission_is_indeterminate). Every other predicate
    /// delegates
    /// through it, so wrapping an error never changes what a caller already decided. Carries no
    /// part of the message.
    #[error("{source}")]
    Indeterminate {
        #[source]
        source: Box<SmtpError>,
    },
}

impl SmtpError {
    /// The retry predicate a backoff loop branches on: a dropped connection, a timeout and a 4xx
    /// are worth retrying; a rejected credential or address is not.
    pub fn is_transient(&self) -> bool {
        matches!(
            self.beneath(),
            Self::Disconnected { .. }
                | Self::TimedOut { .. }
                | Self::Io { .. }
                | Self::Transient { .. }
        )
    }

    /// Whether the message payload may already have reached the server (ADR 0021). Asked before
    /// `is_transient`:
    /// retrying a send that was in fact accepted delivers a second copy nothing can recall.
    /// False means nothing can have been submitted: the server answered with a refusal, or the
    /// transport failed before the
    /// payload began. True means the client stopped hearing from the server after the first byte of
    /// the body and before the
    /// reply to the terminating `.` was read; "never arrived" and "arrived but unacknowledged" are
    /// indistinguishable there.
    /// `DATA` itself is not that moment.
    /// [`SmtpSession::send_message`](crate::session::SmtpSession::send_message) tags the failures
    /// raised inside it.
    pub fn submission_is_indeterminate(&self) -> bool {
        matches!(self, Self::Indeterminate { .. })
    }

    /// Only a failure the client raised is tagged: a reply the server sent says what it did with
    /// the message, and wrapping those would make Postio refuse to retry every refused message.
    pub(crate) fn once_the_payload_was_on_the_wire(self) -> Self {
        match self {
            Self::Disconnected { .. }
            | Self::TimedOut { .. }
            | Self::Io { .. }
            | Self::Cancelled => Self::Indeterminate {
                source: Box::new(self),
            },
            answered => answered,
        }
    }

    fn beneath(&self) -> &Self {
        match self {
            Self::Indeterminate { source } => source,
            other => other,
        }
    }

    pub fn is_authentication_failure(&self) -> bool {
        matches!(self.beneath(), Self::Auth { .. })
    }

    pub fn rejected_recipient(&self) -> Option<&str> {
        match self.beneath() {
            Self::RecipientRejected { recipient, .. } => Some(recipient),
            _ => None,
        }
    }
}
