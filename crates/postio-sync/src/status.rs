//! The status-line state for an account, a throttled sequence over `Link` transitions and
//! `initial::Progress` batches. Translating to `postio-core` events is done above this crate.
//! Several passes run at once, so the foremost (earliest-started, by `sync_priority`) is reported
//! and only the last to finish moves the status to `Idle`.
//! Link transitions pass through; progress is throttled on a caller-supplied clock, except the
//! batch that finishes the pass.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_model::MailboxId;

use crate::connect::{Blocker, Link};
use crate::initial::Progress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncProgress {
    pub mailbox: MailboxId,
    pub done: u32,
    pub total: u32,
}

impl SyncProgress {
    /// A pass with no target (`target == 0`) counts as complete rather than dividing by zero.
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

/// Where an account's sync stands. `Idle` and `Syncing` carry the last completed sync's timestamp
/// for "last sync 12s".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SyncStatus {
    Offline,
    #[default]
    Connecting,
    Idle {
        last_sync: Option<DateTime<Utc>>,
    },
    Syncing {
        mailbox: MailboxId,
        progress: Option<SyncProgress>,
        last_sync: Option<DateTime<Utc>>,
    },
    Error {
        reason: String,
        needs_credentials: bool,
        no_password: bool,
    },
}

#[derive(Debug)]
pub struct StatusTracker {
    status: SyncStatus,
    last_sync: Option<DateTime<Utc>>,
    last_progress_at: Option<DateTime<Utc>>,
    min_progress_interval: TimeDelta,
    /// Passes started and not finished, in start order: `in_flight[0]` is the foremost. At most one
    /// per sync lane.
    in_flight: Vec<MailboxId>,
    /// The link's state while the account cannot reach its server: a pass ending then is not the
    /// account going idle.
    link_down: Option<SyncStatus>,
}

/// Minimum interval between progress reports: inside "updating live" and outside "flooding the UI".
const DEFAULT_MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

impl Default for StatusTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusTracker {
    pub fn new() -> Self {
        Self::with_min_progress_interval(DEFAULT_MIN_PROGRESS_INTERVAL)
    }

    pub fn with_min_progress_interval(interval: Duration) -> Self {
        Self {
            status: SyncStatus::default(),
            last_sync: None,
            last_progress_at: None,
            min_progress_interval: TimeDelta::from_std(interval).unwrap_or(TimeDelta::zero()),
            in_flight: Vec::new(),
            link_down: None,
        }
    }

    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    pub fn last_sync(&self) -> Option<DateTime<Utc>> {
        self.last_sync
    }

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
                no_password: matches!(blocker, Blocker::NoPassword(_)),
            },
        };
        self.link_down = match link {
            Link::Online { .. } => None,
            _ => Some(self.status.clone()),
        };
        self.status.clone()
    }

    pub fn on_sync_started(&mut self, mailbox: MailboxId) -> SyncStatus {
        if !self.in_flight.contains(&mailbox) {
            self.in_flight.push(mailbox);
        }
        // Only the foremost pass resets the throttle; a background folder starting must not let its
        // next batch through early.
        if self.foremost() == Some(mailbox) {
            self.last_progress_at = None;
        }
        self.status = SyncStatus::Syncing {
            mailbox: self.foremost().unwrap_or(mailbox),
            progress: None,
            last_sync: self.last_sync,
        };
        self.keep_the_link_down();
        self.status.clone()
    }

    fn foremost(&self) -> Option<MailboxId> {
        self.in_flight.first().copied()
    }

    /// Returns `None` when the update is inside the throttle window and the pass is not complete;
    /// the batch that finishes the pass is never dropped.
    pub fn on_progress(&mut self, progress: Progress, now: DateTime<Utc>) -> Option<SyncStatus> {
        let progress = SyncProgress::from(progress);
        if self.link_down.is_some() {
            return None;
        }
        // A batch from a non-foremost pass would make the line flicker between folders.
        if self
            .foremost()
            .is_some_and(|foremost| foremost != progress.mailbox)
        {
            return None;
        }
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

    /// Only the last pass in flight moves the status to `Idle` and stamps `last_sync`; otherwise
    /// the line goes to the next pass in priority order.
    /// A mailbox never seen starting leaves the set alone, so a requeued or cancelled pass reported
    /// twice cannot empty it.
    pub fn on_sync_finished(&mut self, mailbox: MailboxId, at: DateTime<Utc>) -> SyncStatus {
        let was_foremost = self.foremost() == Some(mailbox);
        self.in_flight.retain(|&in_flight| in_flight != mailbox);

        match self.foremost() {
            Some(mailbox) => {
                // Reset the throttle so the new foremost pass's next batch reports at once.
                if was_foremost {
                    self.last_progress_at = None;
                }
                self.status = SyncStatus::Syncing {
                    mailbox,
                    progress: None,
                    last_sync: self.last_sync,
                };
            }
            None => {
                self.last_sync = Some(at);
                self.last_progress_at = None;
                self.status = SyncStatus::Idle {
                    last_sync: self.last_sync,
                };
            }
        }
        self.keep_the_link_down();
        self.status.clone()
    }

    /// A pass that stopped unfinished leaves the in-flight set but does not stamp `last_sync`
    /// (T220): an interrupted first pass must not read as finished.
    pub fn on_sync_abandoned(&mut self, mailbox: MailboxId) -> SyncStatus {
        let was_foremost = self.foremost() == Some(mailbox);
        self.in_flight.retain(|&in_flight| in_flight != mailbox);
        match self.foremost() {
            Some(mailbox) => {
                if was_foremost {
                    self.last_progress_at = None;
                }
                self.status = SyncStatus::Syncing {
                    mailbox,
                    progress: None,
                    last_sync: self.last_sync,
                };
            }
            None => {
                self.last_progress_at = None;
                self.status = SyncStatus::Idle {
                    last_sync: self.last_sync,
                };
            }
        }
        self.keep_the_link_down();
        self.status.clone()
    }

    /// A wave's lanes outlive the link: the first to fail takes it down and the rest end after it,
    /// and each used to hand the line to Idle, which read as Online (#2016).
    fn keep_the_link_down(&mut self) {
        if let Some(down) = &self.link_down {
            self.status = down.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

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
    fn an_abandoned_pass_is_not_a_completed_sync() {
        let mut tracker = StatusTracker::new();
        let mailbox = MailboxId::new(1);
        tracker.on_sync_started(mailbox);
        let status = tracker.on_sync_abandoned(mailbox);
        assert_eq!(status, SyncStatus::Idle { last_sync: None });
        tracker.on_sync_started(mailbox);
        let status = tracker.on_sync_finished(mailbox, at(5));
        assert_eq!(
            status,
            SyncStatus::Idle {
                last_sync: Some(at(5))
            }
        );
    }

    #[test]
    fn a_pass_that_ends_after_the_link_dropped_leaves_the_link_down() {
        let mut tracker = StatusTracker::new();
        tracker.on_link(&Link::Online { since: at(0) });
        let (inbox, archive) = (MailboxId::new(1), MailboxId::new(2));
        tracker.on_sync_started(inbox);
        tracker.on_sync_started(archive);
        let waiting = Link::Waiting {
            attempts: 1,
            retry_at: at(2),
        };
        assert_eq!(tracker.on_link(&waiting), SyncStatus::Connecting);

        let junk = MailboxId::new(3);
        assert_eq!(tracker.on_sync_started(junk), SyncStatus::Connecting);
        assert_eq!(tracker.on_progress(progress(inbox, 5, 10), at(1)), None);
        assert_eq!(tracker.on_sync_abandoned(junk), SyncStatus::Connecting);
        assert_eq!(tracker.on_sync_abandoned(inbox), SyncStatus::Connecting);
        assert_eq!(
            tracker.on_sync_finished(archive, at(1)),
            SyncStatus::Connecting
        );
        assert_eq!(tracker.status(), &SyncStatus::Connecting);
        assert_eq!(
            tracker.last_sync(),
            Some(at(1)),
            "a pass that finished still finished"
        );

        assert_eq!(
            tracker.on_link(&Link::Online { since: at(3) }),
            SyncStatus::Idle {
                last_sync: Some(at(1))
            }
        );
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
                no_password: false,
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
        let status = tracker.on_sync_finished(mailbox, at(2));

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

        tracker
            .on_progress(progress(mailbox, 1, 10), at(0))
            .expect("the first batch always reports");
        assert_eq!(
            tracker.on_progress(progress(mailbox, 2, 10), at(0)),
            None,
            "a batch that lands before the interval elapses reports nothing new"
        );

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

        let status = tracker
            .on_progress(progress(mailbox, 10, 10), at(1))
            .expect("completion must be reported even inside the throttle window");
        assert!(matches!(status, SyncStatus::Syncing { progress: Some(p), .. } if p.is_complete()));
    }

    #[test]
    fn a_pass_finishing_while_another_runs_leaves_the_account_syncing() {
        let mut tracker = StatusTracker::new();
        let inbox = MailboxId::new(1);
        let archive = MailboxId::new(2);

        tracker.on_sync_started(inbox);
        tracker.on_sync_started(archive);
        let status = tracker.on_sync_finished(archive, at(1));

        assert!(
            matches!(status, SyncStatus::Syncing { mailbox, .. } if mailbox == inbox),
            "INBOX is still being fetched, so the account is still syncing: {status:?}"
        );
        assert_eq!(
            tracker.last_sync(),
            None,
            "and nothing has finished from the user's point of view yet"
        );
    }

    #[test]
    fn the_last_pass_to_finish_is_what_makes_the_account_idle() {
        let mut tracker = StatusTracker::new();
        let inbox = MailboxId::new(1);
        let archive = MailboxId::new(2);

        tracker.on_sync_started(inbox);
        tracker.on_sync_started(archive);
        tracker.on_sync_finished(archive, at(1));
        let status = tracker.on_sync_finished(inbox, at(2));

        assert_eq!(
            status,
            SyncStatus::Idle {
                last_sync: Some(at(2))
            }
        );
        assert_eq!(tracker.last_sync(), Some(at(2)));
    }

    #[test]
    fn the_status_line_follows_the_pass_the_user_is_waiting_on() {
        let mut tracker = StatusTracker::with_min_progress_interval(Duration::ZERO);
        let inbox = MailboxId::new(1);
        let archive = MailboxId::new(2);

        tracker.on_sync_started(inbox);
        tracker.on_sync_started(archive);

        assert_eq!(
            tracker.on_progress(progress(archive, 900, 40_000), at(1)),
            None,
            "a batch from a background folder tells the status line nothing"
        );

        let status = tracker
            .on_progress(progress(inbox, 200, 1_000), at(1))
            .expect("the foremost pass reports");
        assert!(
            matches!(status, SyncStatus::Syncing { mailbox, .. } if mailbox == inbox),
            "{status:?}"
        );
    }

    #[test]
    fn the_next_pass_takes_over_the_status_line_when_the_foremost_one_ends() {
        let mut tracker = StatusTracker::with_min_progress_interval(Duration::ZERO);
        let inbox = MailboxId::new(1);
        let archive = MailboxId::new(2);

        tracker.on_sync_started(inbox);
        tracker.on_sync_started(archive);
        tracker.on_sync_finished(inbox, at(1));

        let status = tracker
            .on_progress(progress(archive, 900, 40_000), at(2))
            .expect("the archive is the foremost pass now");
        assert!(
            matches!(status, SyncStatus::Syncing { mailbox, .. } if mailbox == archive),
            "{status:?}"
        );
    }

    #[test]
    fn a_pass_that_was_never_started_cannot_finish_the_account() {
        let mut tracker = StatusTracker::new();
        let inbox = MailboxId::new(1);

        tracker.on_sync_started(inbox);
        let status = tracker.on_sync_finished(MailboxId::new(99), at(1));

        assert!(
            matches!(status, SyncStatus::Syncing { mailbox, .. } if mailbox == inbox),
            "{status:?}"
        );
    }

    #[test]
    fn starting_a_new_pass_resets_the_progress_throttle() {
        let mailbox = MailboxId::new(1);
        let mut tracker = StatusTracker::with_min_progress_interval(Duration::from_secs(60));
        tracker.on_sync_started(mailbox);
        tracker
            .on_progress(progress(mailbox, 1, 10), at(0))
            .expect("first report of the first pass");

        tracker.on_sync_finished(mailbox, at(1));
        tracker.on_sync_started(mailbox);

        assert!(
            tracker
                .on_progress(progress(mailbox, 1, 10), at(1))
                .is_some(),
            "a new pass's first batch must not be throttled by the previous pass's timing"
        );
    }
}
