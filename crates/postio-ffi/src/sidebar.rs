//! The sidebar's footer, which is a sentence rather than a widget.

/// What an account is doing, as the footer ranks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ActivityFfi {
    /// The machine has no connection.
    Offline,
    /// A pass is running now.
    Syncing,
    /// Nothing is running.
    Idle,
}

impl From<ActivityFfi> for postio_ui::sidebar::Activity {
    fn from(activity: ActivityFfi) -> Self {
        match activity {
            ActivityFfi::Offline => Self::Offline,
            ActivityFfi::Syncing => Self::Syncing,
            ActivityFfi::Idle => Self::Idle,
        }
    }
}

/// The footer line: `idle · synced 40s`.
///
/// A free function because it is a rendering of two facts the frontend
/// already holds — what the connection is doing, and when a pass last
/// finished — rather than a question about a session. The wording is
/// `postio_ui::sidebar`'s so both frontends' footers read the same.
#[uniffi::export]
pub fn sidebar_status(activity: ActivityFfi, since_seconds: Option<i64>) -> String {
    postio_ui::sidebar::status(
        activity.into(),
        // A clock that has gone backwards -- a machine that slept, an NTP
        // correction -- reads as "just now" rather than as a negative age.
        since_seconds.map(|seconds| seconds.max(0) as u64),
    )
}
