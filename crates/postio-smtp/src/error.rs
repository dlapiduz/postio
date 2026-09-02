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

    /// A failure raised once the message payload had begun being written.
    ///
    /// Not something to match on and not something to construct: it is how
    /// the `DATA` path carries the one fact SMTP itself cannot report — that
    /// the bytes had started leaving — out to
    /// [`submission_is_indeterminate`](Self::submission_is_indeterminate).
    /// Every other predicate here delegates straight through it, so wrapping
    /// an error changes what a caller can *ask*, never what it already
    /// decided. That is the property this module's header promises and the
    /// only reason a new variant is safe to add.
    ///
    /// It carries the failure it wraps and nothing else — in particular no
    /// part of the message, which is what keeps these errors safe to log
    /// verbatim.
    #[error("{source}")]
    Indeterminate {
        /// What actually went wrong.
        #[source]
        source: Box<SmtpError>,
    },
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
            self.beneath(),
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
    /// **The two predicates are orthogonal, and this one is asked first.** A
    /// connection lost while the payload was going out is still exactly the
    /// kind of failure a backoff loop retries — which is the trap: `4xx`,
    /// a reset socket and a timeout are all "try again later" right up until
    /// the moment trying again means sending the message twice.
    ///
    /// **False means nothing of the message can have been submitted.** Either
    /// the server answered — a rejected credential, sender, recipient or
    /// message, an unreadable reply, a `4xx`, all of which mean the
    /// transaction ended in a refusal the client witnessed — or the transport
    /// failed *before* the payload began being written, in which case there
    /// is nothing on the wire to have been accepted. Both are safely
    /// retryable, and a caller may undo whatever it did in anticipation.
    ///
    /// **True means the client stopped hearing from the server after the
    /// payload had begun.** `DATA` is not that moment: it asks permission,
    /// and the message goes out only after the server's `354`. The window
    /// opens with the first byte of the body and closes when the reply to the
    /// terminating `.` is read; inside it, "it never arrived" and "it arrived
    /// and the acknowledgement did not" are indistinguishable, and no amount
    /// of protocol can separate them. [`SmtpSession::send_message`
    /// ](crate::session::SmtpSession::send_message) is what knows where that
    /// window is, and it tags the failures raised inside it.
    ///
    /// Until #673 this was approximated by variant: every transport failure
    /// anywhere in the session was treated as though it might have been that
    /// one, because the costs are not symmetric. That was safe and far too
    /// broad — a connection dropped during `EHLO` stranded a queued message
    /// that nothing had ever tried to send.
    pub fn submission_is_indeterminate(&self) -> bool {
        matches!(self, Self::Indeterminate { .. })
    }

    /// Tags a failure as having been raised once the payload was on the wire.
    ///
    /// Only a failure the *client* raised is tagged. A reply the server sent
    /// — a rejection, a `4xx` — is the server saying what it did with the
    /// message, so being past the payload boundary does not make it unknown;
    /// wrapping those would turn every refused message into one Postio
    /// refuses to retry.
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

    /// The failure itself, looking through an [`Indeterminate`
    /// ](Self::Indeterminate) wrapper.
    ///
    /// Every predicate but [`submission_is_indeterminate`
    /// ](Self::submission_is_indeterminate) answers from here, so tagging an
    /// error cannot change an answer a caller already relied on.
    fn beneath(&self) -> &Self {
        match self {
            Self::Indeterminate { source } => source,
            other => other,
        }
    }

    /// Whether the user has to supply a new password before anything will
    /// work.
    pub fn is_authentication_failure(&self) -> bool {
        matches!(self.beneath(), Self::Auth { .. })
    }

    /// The recipient address a permanent rejection named, when this is one.
    pub fn rejected_recipient(&self) -> Option<&str> {
        match self.beneath() {
            Self::RecipientRejected { recipient, .. } => Some(recipient),
            _ => None,
        }
    }
}
