//! What an account's status line shows, and how often it is allowed to change.
//!
//! # Not the event that reaches the UI
//!
//! `postio-core`'s own event stream already has the shape the sidebar
//! consumes — `Event::ConnectionChanged` and `Event::SyncProgress` — and its
//! own doc comment says why they are not this module: *"A summary for the
//! status line, deliberately not the sync engine's internal state machine —
//! `postio-core` should not have to change when that does."* [`SyncStatus`]
//! is that internal state machine. Something above this crate translates it
//! into `postio-core` events; this module's job stops at producing a correct,
//! throttled sequence of them.
//!
//! # Two different kinds of infrequent
//!
//! A connection transition ([`Link`]) is already coalesced upstream —
//! [`crate::connect::Supervisor::poll`] only returns a value when something
//! changed — so [`StatusTracker::on_link`] passes every one of those through.
//! Progress is different: [`crate::initial::sync_mailbox`] calls its
//! callback once per *batch*, and a small batch size against a fast
//! connection can mean many updates a second, far more than a status line
//! needs to redraw for. [`StatusTracker::on_progress`] is where that gets
//! throttled, on a caller-supplied clock so the policy is testable without a
//! timer — the same shape [`crate::connect::ReconnectPolicy`] uses for jitter.
//! The one update never dropped is the batch that finishes the pass: an
//! account is not allowed to sit at 90% because the last update before
//! completion happened to land inside the throttle window.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_model::MailboxId;

use crate::connect::Link;
use crate::initial::Progress;

/// How far a batch fetch has gotten inside one mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncProgress {
    /// The mailbox this progress describes.
    pub mailbox: MailboxId,
    /// Messages fetched so far this pass.
    pub done: u32,
    /// The pass's target, per [`crate::initial::Progress::target`].
    pub total: u32,
}

impl SyncProgress {
    /// Whether this batch was the one that finished the pass.
    ///
    /// A pass with no target at all (an empty mailbox, `target == 0`) counts
    /// as complete rather than dividing by zero forever.
    pub fn is_complete(&self) -> bool {
        self.done >= self.total
    }
}

impl From<Progress> for SyncProgress {
    fn from(progress: Progress) -> Self {
        Self {
            mailbox: progress.mailbox_id,
            done: progress.fetched,
            total: progress.target,
        }
    }
}

/// Where an account's sync stands right now.
///
/// The five states CLAUDE.md's design canvas needs for the sidebar's status
/// line — `idle`, `connecting`, `syncing(progress)`, `offline`, `error(reason)`
/// — map onto this one for one, with [`SyncStatus::Idle`] and
/// [`SyncStatus::Syncing`] both carrying the last completed sync's timestamp
/// so "last sync 12s" can be rendered without a second lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    /// The machine has no network. See [`Link::Offline`].
    Offline,
    /// Trying to connect, or waiting out a backoff. See [`Link::Waiting`].
    Connecting,
    /// Connected, nothing in flight right now.
    Idle {
        /// When a sync last completed, if ever.
        last_sync: Option<DateTime<Utc>>,
    },
    /// Connected and actively fetching.
    Syncing {
        /// The mailbox being synchronized.
        mailbox: MailboxId,
        /// How far this pass has gotten, once the first batch has committed.
        progress: Option<SyncProgress>,
        /// When a sync last completed *before this one*, if ever.
        last_sync: Option<DateTime<Utc>>,
    },
    /// Stopped, and waiting for the user. See [`Link::Blocked`].
    Error {
        /// Why, phrased for the user — [`crate::connect::Blocker::reason`].
        reason: String,
        /// Whether the user has to supply a new password.
        needs_credentials: bool,
    },
}

/// A never-connected account's status: not yet even trying.
impl Default for SyncStatus {
    fn default() -> Self {
        Self::Connecting
    }
}

/// Turns [`Link`] transitions and [`Progress`] batches into a throttled
/// sequence of [`SyncStatus`] values for one account.
#[derive(Debug)]
pub struct StatusTracker {
    status: SyncStatus,
    last_sync: Option<DateTime<Utc>>,
    last_progress_at: Option<DateTime<Utc>>,
    min_progress_interval: TimeDelta,
}

/// How often [`StatusTracker::on_progress`] is allowed to report, at most.
///
/// A quarter of a second is well inside the perception threshold for
/// "updating live" and well outside "flooding the UI" for a fast local
/// connection fetching small batches.
const DEFAULT_MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

impl Default for StatusTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusTracker {
    /// A tracker for an account that has not connected yet.
    pub fn new() -> Self {
        Self::with_min_progress_interval(DEFAULT_MIN_PROGRESS_INTERVAL)
    }

    /// A tracker with a non-default progress throttle, mainly for tests that
    /// want to see every batch, or a caller with a slower UI budget.
    pub fn with_min_progress_interval(interval: Duration) -> Self {
        Self {
            status: SyncStatus::default(),
            last_sync: None,
            last_progress_at: None,
            min_progress_interval: TimeDelta::from_std(interval).unwrap_or(TimeDelta::zero()),
        }
    }

    /// The status as of the last update.
    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    /// When a sync last completed, if ever.
    pub fn last_sync(&self) -> Option<DateTime<Utc>> {
        self.last_sync
    }

    /// Applies a connection transition. Always reported: [`Link`] transitions
    /// are already coalesced by [`crate::connect::Supervisor::poll`], so there
    /// is nothing left here to throttle.
    pub fn on_link(&mut self, link: &Link) -> SyncStatus {
        self.status = match link {
            Link::Offline => SyncStatus::Offline,
            Link::Waiting { .. } => SyncStatus::Connecting,
            Link::Online { .. } => SyncStatus::Idle {
                last_sync: self.last_sync,
            },
            Link::Blocked(blocker) => SyncStatus::Error {
                reason: blocker.reason().to_owned(),
                needs_credentials: blocker.needs_credentials(),
            },
        };
        self.status.clone()
    }

    /// Marks a sync pass starting on `mailbox`. Always reported: it happens
    /// once per pass, not once per batch.
    pub fn on_sync_started(&mut self, mailbox: MailboxId) -> SyncStatus {
        self.last_progress_at = None;
        self.status = SyncStatus::Syncing {
            mailbox,
            progress: None,
            last_sync: self.last_sync,
        };
        self.status.clone()
    }

    /// Applies one committed batch's progress.
    ///
    /// Returns `None` when the update arrived before
    /// [`min_progress_interval`](Self::with_min_progress_interval) elapsed
    /// and the pass is not yet complete — the caller has nothing new to tell
    /// the UI. The batch that finishes the pass is never dropped.
    pub fn on_progress(&mut self, progress: Progress, now: DateTime<Utc>) -> Option<SyncStatus> {
        let progress = SyncProgress::from(progress);
        let due = self
            .last_progress_at
            .is_none_or(|at| now - at >= self.min_progress_interval);
        if !due && !progress.is_complete() {
            return None;
        }
        self.last_progress_at = Some(now);
        self.status = SyncStatus::Syncing {
            mailbox: progress.mailbox,
            progress: Some(progress),
            last_sync: self.last_sync,
        };
        Some(self.status.clone())
    }

    /// Marks a sync pass finished at `at`. Always reported, and always moves
    /// the status to [`SyncStatus::Idle`] — a caller that reconnects or fails
    /// afterwards reports that separately through [`Self::on_link`].
    pub fn on_sync_finished(&mut self, at: DateTime<Utc>) -> SyncStatus {
        self.last_sync = Some(at);
        self.last_progress_at = None;
        self.status = SyncStatus::Idle {
            last_sync: self.last_sync,
        };
        self.status.clone()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::connect::Blocker;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second as i64)
    }

    fn progress(mailbox: MailboxId, fetched: u32, target: u32) -> Progress {
        Progress {
            mailbox_id: mailbox,
            fetched,
            target,
        }
    }

    #[test]
    fn a_fresh_tracker_has_not_connected_yet() {
        let tracker = StatusTracker::new();
        assert_eq!(tracker.status(), &SyncStatus::Connecting);
        assert_eq!(tracker.last_sync(), None);
    }

    #[test]
    fn link_transitions_map_onto_the_five_canvas_states() {
        let mut tracker = StatusTracker::new();

        assert_eq!(tracker.on_link(&Link::Offline), SyncStatus::Offline);
        assert_eq!(
            tracker.on_link(&Link::Waiting {
                attempts: 1,
                retry_at: at(1),
            }),
            SyncStatus::Connecting
        );
        assert_eq!(
            tracker.on_link(&Link::Online { since: at(0) }),
            SyncStatus::Idle { last_sync: None }
        );
    }

    #[test]
    fn an_error_status_carries_an_actionable_reason() {
        let mut tracker = StatusTracker::new();

        let status = tracker.on_link(&Link::Blocked(Blocker::Authentication(
            "the server rejected the app-specific password".to_owned(),
        )));

        assert_eq!(
            status,
            SyncStatus::Error {
                reason: "the server rejected the app-specific password".to_owned(),
                needs_credentials: true,
            }
        );
    }

    #[test]
    fn a_completed_pass_is_idle_with_a_last_sync_time() {
        let mut tracker = StatusTracker::new();
        let mailbox = MailboxId::new(1);

        tracker.on_link(&Link::Online { since: at(0) });
        tracker.on_sync_started(mailbox);
        tracker.on_progress(progress(mailbox, 5, 5), at(1));
        let status = tracker.on_sync_finished(at(2));

        assert_eq!(
            status,
            SyncStatus::Idle {
                last_sync: Some(at(2))
            }
        );
        assert_eq!(tracker.last_sync(), Some(at(2)));
    }

    #[test]
    fn high_frequency_progress_is_throttled() {
        let mailbox = MailboxId::new(1);
        let mut tracker = StatusTracker::with_min_progress_interval(Duration::from_secs(1));
        tracker.on_sync_started(mailbox);

        // The first batch of the pass always establishes the baseline.
        tracker
            .on_progress(progress(mailbox, 1, 10), at(0))
            .expect("the first batch always reports");
        // Still inside the window.
        assert_eq!(
            tracker.on_progress(progress(mailbox, 2, 10), at(0)),
            None,
            "a batch that lands before the interval elapses reports nothing new"
        );

        // A whole second later: due again.
        let status = tracker
            .on_progress(progress(mailbox, 3, 10), at(1))
            .expect("a batch after the interval must report");
        assert_eq!(
            status,
            SyncStatus::Syncing {
                mailbox,
                progress: Some(SyncProgress {
                    mailbox,
                    done: 3,
                    total: 10
                }),
                last_sync: None,
            }
        );
    }

    #[test]
    fn the_batch_that_finishes_a_pass_is_never_throttled_away() {
        let mailbox = MailboxId::new(1);
        let mut tracker = StatusTracker::with_min_progress_interval(Duration::from_secs(60));
        tracker.on_sync_started(mailbox);

        tracker
            .on_progress(progress(mailbox, 1, 10), at(0))
            .expect("the first batch always reports");

        // Well inside the throttle window, but it is the last batch.
        let status = tracker
            .on_progress(progress(mailbox, 10, 10), at(1))
            .expect("completion must be reported even inside the throttle window");
        assert!(matches!(status, SyncStatus::Syncing { progress: Some(p), .. } if p.is_complete()));
    }

    #[test]
    fn starting_a_new_pass_resets_the_progress_throttle() {
        let mailbox = MailboxId::new(1);
        let mut tracker = StatusTracker::with_min_progress_interval(Duration::from_secs(60));
        tracker.on_sync_started(mailbox);
        tracker
            .on_progress(progress(mailbox, 1, 10), at(0))
            .expect("first report of the first pass");

        tracker.on_sync_finished(at(1));
        tracker.on_sync_started(mailbox);

        assert!(
            tracker
                .on_progress(progress(mailbox, 1, 10), at(1))
                .is_some(),
            "a new pass's first batch must not be throttled by the previous pass's timing"
        );
    }
}
