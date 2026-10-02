//! The sidebar's footer, which is a sentence rather than a widget.

/// What an account is doing, as the footer ranks it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ActivityFfi {
    /// The machine has no connection.
    Offline,
    /// A pass is running now.
    Syncing,
    /// The account cannot sign in, and why.
    ///
    /// Distinct from `Offline`, which is the *machine's* state. This one is
    /// the account's and is the one that needs a person — and it had no
    /// spelling here at all, so `ConnectionState::Failing`'s reason was
    /// discarded at the boundary and an expired password read as
    /// `idle · synced 40s` forever.
    Failing {
        /// What the server or the keyring said, phrased for the user.
        reason: String,
    },
    /// Nothing is running.
    Idle,
}

impl From<ActivityFfi> for postio_ui::sidebar::Activity {
    fn from(activity: ActivityFfi) -> Self {
        match activity {
            ActivityFfi::Offline => Self::Offline,
            ActivityFfi::Syncing => Self::Syncing,
            ActivityFfi::Failing { reason } => Self::Failing { reason },
            ActivityFfi::Idle => Self::Idle,
        }
    }
}

/// What a failing account's footer says, by what kind of failure it is.
///
/// The wording is `postio_ui::sidebar::failing_because`'s, so both platforms
/// say the same thing about the same failure. `FailureReasonFfi` is a
/// classification and this is the sentence for it — the server's own text,
/// when there is any, arrives separately as a `Notice`.
#[uniffi::export]
pub fn failure_sentence(reason: crate::FailureReasonFfi) -> String {
    postio_ui::sidebar::failing_because(reason.into()).to_owned()
}

/// The footer line: `idle · synced 40s`.
///
/// A free function because it is a rendering of two facts the frontend
/// already holds — what the connection is doing, and when a pass last
/// finished — rather than a question about a session. The wording is
/// `postio_ui::sidebar`'s so both frontends' footers read the same.
#[uniffi::export]
pub fn sidebar_status(activity: ActivityFfi, since_seconds: Option<i64>, has_mail: bool) -> String {
    postio_ui::sidebar::status(
        activity.into(),
        // A clock that has gone backwards -- a machine that slept, an NTP
        // correction -- reads as "just now" rather than as a negative age.
        since_seconds.map(|seconds| seconds.max(0) as u64),
        has_mail,
    )
}
