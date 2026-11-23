//! The sync status line's words and the tracker that folds core events into them, shared by both
//! frontends.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use postio_core::{ConnectionState, Event};
use postio_model::mailbox::Mailbox;
use postio_model::{AccountId, MailboxId};

const PROTOCOL: &str = "imap";

/// `last_sync` is an `Instant` because it shows an age, which must not jump when the system clock
/// is corrected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStatus {
    pub state: ConnectionState,
    pub last_sync: Option<Instant>,
    pub progress: Option<(u32, u32)>,
    /// Kept apart from `progress`: a list still arriving cannot be read, while mail whose text is
    /// arriving can (#74).
    pub backfill: Option<(u32, u32)>,
    pub detail: Option<String>,
    /// `None` until measured; arrives on `BackfillProgress` beside the counts (#411, ADR 0017).
    pub footprint: Option<postio_core::event::MailFootprint>,
    /// The folder's own fact: only that folder's next completed pass clears it, no connection
    /// change does (#1731).
    pub failing: Vec<(MailboxId, String)>,
}

impl Default for SyncStatus {
    fn default() -> Self {
        SyncStatus {
            state: ConnectionState::Offline,
            last_sync: None,
            progress: None,
            backfill: None,
            detail: None,
            footprint: None,
            failing: Vec::new(),
        }
    }
}

impl SyncStatus {
    pub fn lines(&self, now: Instant) -> (String, String) {
        (
            format!("{} · {PROTOCOL}", self.state_word()),
            self.detail_line(now),
        )
    }

    fn state_word(&self) -> String {
        match self.state {
            ConnectionState::Offline => "offline".to_string(),
            ConnectionState::Connecting => "connecting".to_string(),
            ConnectionState::Failing { .. } => "error".to_string(),
            ConnectionState::Online if self.syncing().is_some() => "syncing".to_string(),
            // "downloading", not "syncing" (the list) or "idle" (#74).
            ConnectionState::Online if self.filling().is_some() => "downloading".to_string(),
            ConnectionState::Online => "idle".to_string(),
        }
    }

    /// One question both lines answer from, so they cannot disagree. `progress` is `Some` exactly
    /// while a pass is in flight.
    fn syncing(&self) -> Option<u32> {
        match self.progress {
            Some((_, 0)) => None,
            Some((done, total)) if done < total => Some(done),
            _ => None,
        }
    }

    /// `None` once the queue drains, so a finished backfill does not stick at `2000 of 2000`.
    pub fn backfill_running(&self) -> Option<(u32, u32)> {
        self.filling()
    }

    fn filling(&self) -> Option<(u32, u32)> {
        match self.backfill {
            Some((_, 0)) => None,
            Some((done, total)) if done < total => Some((done, total)),
            _ => None,
        }
    }

    /// The reason wins over "last sync 4h" when the connection is failing.
    fn detail_line(&self, now: Instant) -> String {
        if matches!(self.state, ConnectionState::Failing { .. })
            && let Some(detail) = &self.detail
        {
            return detail.clone();
        }
        // A running pass says what it has fetched, not a percentage: the denominator is `UIDNEXT -
        // 1`, which expunged messages leave gaps in.
        // No thousands separator, matching the folder counts.
        if let Some(fetched) = self.syncing() {
            return format!("fetched {fetched}");
        }
        // A backfill knows its real denominator, so it can say "of".
        if let Some((done, total)) = self.filling() {
            return format!("mail {done} of {total}");
        }
        match self.last_sync {
            Some(at) => format!("last sync {}", age(now.saturating_duration_since(at))),
            None => "never synced".to_string(),
        }
    }
}

pub fn age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// The failure reason arrives as `Event::Error` beside the typed `ConnectionState::Failing`; the
/// tracker keeps the last one and clears it on recovery.
#[derive(Clone, Debug, Default)]
pub struct SyncTracker {
    status: SyncStatus,
    reason: Option<String>,
}

/// One tracker per account, so no account's server speaks for another (ADR 0005 Q10).
/// An `Event::Error` naming no account goes to the account on screen.
#[derive(Clone, Debug, Default)]
pub struct Trackers {
    per_account: std::collections::BTreeMap<AccountId, SyncTracker>,
}

impl Trackers {
    pub fn apply(&mut self, event: &Event, current: Option<AccountId>) -> bool {
        let account = match event {
            Event::ConnectionChanged { account, .. }
            | Event::SyncProgress { account, .. }
            | Event::BackfillProgress { account, .. }
            | Event::MailboxSyncChanged { account, .. }
            | Event::Error {
                account: Some(account),
                ..
            } => Some(*account),
            _ => current,
        };
        let Some(account) = account else {
            return false;
        };
        self.per_account.entry(account).or_default().apply(event)
    }

    /// An account nothing has been heard about is offline.
    pub fn status(&self, account: AccountId) -> SyncStatus {
        self.per_account
            .get(&account)
            .map(|tracker| tracker.status().clone())
            .unwrap_or_default()
    }

    /// Per account: the newest `last_synced_at` in a flat list belongs to whichever account synced
    /// last.
    pub fn note_last_sync(&mut self, account: AccountId, mailboxes: &[Mailbox]) -> bool {
        let theirs: Vec<Mailbox> = mailboxes
            .iter()
            .filter(|mailbox| mailbox.account_id == account)
            .cloned()
            .collect();
        self.per_account
            .entry(account)
            .or_default()
            .note_last_sync(&theirs)
    }

    pub fn failure(&self, mailbox: MailboxId) -> Option<&str> {
        self.per_account.values().find_map(|tracker| {
            tracker
                .status
                .failing
                .iter()
                .find(|(each, _)| *each == mailbox)
                .map(|(_, reason)| reason.as_str())
        })
    }

    /// In the caller's order (the sidebar's, which keys the hues); an unheard-of account still gets
    /// an entry.
    pub fn statuses(&self, accounts: &[AccountId]) -> Vec<(AccountId, SyncStatus)> {
        accounts
            .iter()
            .map(|account| (*account, self.status(*account)))
            .collect()
    }
}

impl SyncTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        let before = self.status.clone();
        match event {
            Event::ConnectionChanged { state, .. } => {
                self.status.state = *state;
                if matches!(state, ConnectionState::Failing { .. }) {
                    self.status.detail = self.reason.clone();
                } else {
                    self.status.detail = None;
                    self.reason = None;
                }
                // A pass's progress ends at a connection boundary, `Online` included: `total` is
                // `UIDNEXT - 1`, which expunged messages leave gaps in, so a finished pass may
                // never reach it.
                self.status.progress = None;
                // `backfill` is not cleared: it spans many IDLE cycles and reconnects (#316);
                // `BackfillProgress` clears it when the queue drains.
            }
            Event::BackfillProgress {
                done,
                total,
                footprint,
                ..
            } => {
                self.status.backfill = Some((*done, *total));
                // Kept when the queue drains: the mail's size is true with or without a backfill.
                if footprint.is_some() {
                    self.status.footprint = *footprint;
                }
                // Drained: clear it, but leave `last_sync`, which means a list pass completed.
                if done >= total {
                    self.status.backfill = None;
                }
            }
            Event::SyncProgress { done, total, .. } => {
                self.status.progress = Some((*done, *total));
                if done >= total {
                    self.status.last_sync = Some(Instant::now());
                    self.status.progress = None;
                }
            }
            Event::MailboxSyncChanged {
                mailbox, failure, ..
            } => {
                let failing = &mut self.status.failing;
                let at = failing.binary_search_by_key(mailbox, |(each, _)| *each);
                match (at, failure) {
                    (Ok(at), Some(reason)) => reason.clone_into(&mut failing[at].1),
                    (Err(at), Some(reason)) => failing.insert(at, (*mailbox, reason.clone())),
                    (Ok(at), None) => {
                        failing.remove(at);
                    }
                    (Err(_), None) => {}
                }
            }
            Event::Error { message, .. } => {
                self.reason = Some(message.clone());
                if matches!(self.status.state, ConnectionState::Failing { .. }) {
                    self.status.detail = Some(message.clone());
                }
            }
            _ => return false,
        }
        self.status != before
    }

    /// An `Instant`, since the stored time is wall-clock and an age must not jump with the clock.
    pub fn note_last_sync(&mut self, mailboxes: &[Mailbox]) -> bool {
        let Some(latest) = mailboxes.iter().filter_map(|m| m.last_synced_at).max() else {
            return false;
        };
        let converted = to_instant(
            latest,
            crate::clock::now().with_timezone(&Utc),
            crate::clock::instant(),
        );
        if converted.is_some() && self.status.last_sync.is_none() {
            self.status.last_sync = converted;
            return true;
        }
        false
    }
}

/// `None` for a time in the future or older than the process: neither fits an `Instant`.
pub fn to_instant(at: DateTime<Utc>, now: DateTime<Utc>, monotonic: Instant) -> Option<Instant> {
    let age = now.signed_duration_since(at).to_std().ok()?;
    monotonic.checked_sub(age)
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use postio_model::mailbox::MailboxRole;

    fn account() -> AccountId {
        AccountId::new(1)
    }

    fn connection(state: ConnectionState) -> Event {
        Event::ConnectionChanged {
            account: account(),
            state,
        }
    }

    fn folder(mailbox: i64, failure: Option<&str>) -> Event {
        Event::MailboxSyncChanged {
            account: account(),
            mailbox: MailboxId::new(mailbox),
            failure: failure.map(str::to_owned),
        }
    }

    /// #1731: a failed folder outlives connection changes until its next pass completes.
    #[test]
    fn a_failed_folder_stays_failed_until_its_next_pass_completes() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        assert!(tracker.apply(&folder(7, Some("The server refused to open Sent."))));
        assert!(!tracker.apply(&folder(7, Some("The server refused to open Sent."))));
        tracker.apply(&connection(ConnectionState::Connecting));
        tracker.apply(&connection(ConnectionState::Online));
        assert_eq!(
            tracker.status().failing,
            vec![(
                MailboxId::new(7),
                "The server refused to open Sent.".to_owned()
            )],
            "a connection change is not that folder's pass completing"
        );

        assert!(tracker.apply(&folder(7, None)));
        assert!(tracker.status().failing.is_empty());
    }

    #[test]
    fn a_failed_folder_is_its_own_accounts() {
        let mut trackers = Trackers::default();
        trackers.apply(&folder(7, Some("No.")), Some(AccountId::new(2)));
        assert_eq!(trackers.status(account()).failing.len(), 1);
        assert!(trackers.status(AccountId::new(2)).failing.is_empty());
    }

    #[test]
    fn a_backfill_moves_the_status_line_and_then_gets_out_of_the_way() {
        let mut tracker = SyncTracker::new();
        assert!(tracker.apply(&Event::ConnectionChanged {
            account: AccountId::new(1),
            state: ConnectionState::Online,
        }));
        assert_eq!(tracker.status().backfill, None);

        assert!(
            tracker.apply(&Event::BackfillProgress {
                account: AccountId::new(1),
                done: 412,
                total: 2000,
                footprint: None,
            }),
            "the status changed and the tracker said it had not"
        );
        assert_eq!(tracker.status().backfill, Some((412, 2000)));

        tracker.apply(&Event::BackfillProgress {
            account: AccountId::new(1),
            done: 2000,
            total: 2000,
            footprint: None,
        });
        assert_eq!(
            tracker.status().backfill,
            None,
            "a queue that has drained is not a backfill in progress"
        );
        assert_eq!(tracker.status().lines(Instant::now()).0, "idle · imap");
    }

    /// #316: a connection announcement mid-backfill must not erase its count.
    #[test]
    fn a_connection_announcement_mid_backfill_does_not_erase_its_count() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::BackfillProgress {
            account: account(),
            done: 412,
            total: 2000,
            footprint: None,
        });
        assert_eq!(
            tracker.status().lines(Instant::now()).0,
            "downloading · imap"
        );

        tracker.apply(&connection(ConnectionState::Online));

        assert_eq!(
            tracker.status().backfill,
            Some((412, 2000)),
            "a connection announcement mid-backfill must not erase its count"
        );
        assert_eq!(
            tracker.status().lines(Instant::now()).0,
            "downloading · imap",
            "the status line must not claim idle while a body is still in flight"
        );
    }

    #[test]
    fn a_backfill_does_not_pretend_to_be_a_sync() {
        // `last_sync` dates a list pass; a backfill finishing must not move it.
        let mut tracker = SyncTracker::new();
        tracker.apply(&Event::ConnectionChanged {
            account: AccountId::new(1),
            state: ConnectionState::Online,
        });
        let before = tracker.status().last_sync;
        tracker.apply(&Event::BackfillProgress {
            account: AccountId::new(1),
            done: 2000,
            total: 2000,
            footprint: None,
        });
        assert_eq!(
            tracker.status().last_sync,
            before,
            "a drained body queue is not a completed sync"
        );
    }

    #[test]
    fn the_status_line_follows_a_connection_all_the_way_round() {
        let mut tracker = SyncTracker::new();
        assert_eq!(tracker.status().state, ConnectionState::Offline);
        assert_eq!(tracker.status().last_sync, None);

        assert!(tracker.apply(&connection(ConnectionState::Connecting)));
        assert_eq!(tracker.status().state, ConnectionState::Connecting);

        assert!(tracker.apply(&connection(ConnectionState::Online)));
        assert!(tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 40,
            total: 100,
        }));
        assert_eq!(tracker.status().progress, Some((40, 100)));

        assert!(tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 100,
            total: 100,
        }));
        assert_eq!(tracker.status().progress, None);
        assert!(tracker.status().last_sync.is_some());
    }

    #[test]
    fn a_failing_connection_carries_the_reason_it_was_given() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&Event::Error {
            message: "the server rejected the password".to_string(),
            account: None,
        });
        tracker.apply(&connection(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        }));
        assert_eq!(
            tracker.status().detail.as_deref(),
            Some("the server rejected the password")
        );

        tracker.apply(&Event::Error {
            message: "the certificate expired".to_string(),
            account: None,
        });
        assert_eq!(
            tracker.status().detail.as_deref(),
            Some("the certificate expired")
        );

        tracker.apply(&connection(ConnectionState::Online));
        assert_eq!(tracker.status().detail, None);

        tracker.apply(&connection(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        }));
        assert_eq!(tracker.status().detail, None);
    }

    #[test]
    fn a_dropped_connection_stops_reporting_progress_it_is_not_making() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 3,
            total: 90,
        });
        tracker.apply(&connection(ConnectionState::Offline));
        assert_eq!(tracker.status().progress, None, "syncing 3% while offline");
    }

    #[test]
    fn a_pass_that_ends_short_of_its_own_total_stops_reporting_a_percentage() {
        // `total` is `UIDNEXT - 1`, which expunged messages leave gaps in, so a finished pass can
        // end below 100%; the idle `Online` must clear the number.
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::SyncProgress {
            account: account(),
            done: 89,
            total: 100,
        });
        assert_eq!(tracker.status().progress, Some((89, 100)), "mid-pass");

        tracker.apply(&connection(ConnectionState::Online));
        assert_eq!(
            tracker.status().progress,
            None,
            "the pass finished; there is no percentage to be a percentage of"
        );
    }

    #[test]
    fn events_the_status_line_is_not_about_change_nothing() {
        let mut tracker = SyncTracker::new();
        assert!(!tracker.apply(&Event::MailboxesChanged { account: account() }));
        assert!(!tracker.apply(&Event::BodyLoaded {
            account: account(),
            message: postio_model::ids::MessageId::new(1),
            sections: postio_core::event::Sections::Text,
        }));
    }

    #[test]
    fn the_last_sync_age_comes_off_the_monotonic_clock() {
        let now = Utc::now();
        let monotonic = Instant::now();

        let hour = to_instant(now - chrono::Duration::hours(1), now, monotonic)
            .expect("an hour is expressible");
        assert!((monotonic.duration_since(hour).as_secs() as i64 - 3600).abs() <= 1);

        assert_eq!(
            to_instant(now + chrono::Duration::hours(1), now, monotonic),
            None
        );
    }

    #[test]
    fn folders_report_when_the_account_last_synced() {
        let synced = |id: i64, at: Option<DateTime<Utc>>| {
            let mut mailbox = Mailbox::new(account(), "INBOX", Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = MailboxRole::Inbox;
            mailbox.last_synced_at = at;
            mailbox
        };
        let now = Utc::now();

        let mut tracker = SyncTracker::new();
        assert!(!tracker.note_last_sync(&[synced(1, None)]), "never synced");
        assert_eq!(tracker.status().last_sync, None);

        assert!(tracker.note_last_sync(&[
            synced(1, Some(now - chrono::Duration::days(2))),
            synced(2, Some(now - chrono::Duration::seconds(12))),
        ]));
        let age = Instant::now().saturating_duration_since(tracker.status().last_sync.unwrap());
        assert!(age.as_secs() <= 13, "the age came out as {age:?}");
    }
}

#[cfg(test)]
mod trackers_tests {
    use super::*;

    const WORK: AccountId = AccountId::new(1);
    const HOME: AccountId = AccountId::new(2);

    #[test]
    fn each_account_keeps_its_own_connection_state() {
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::ConnectionChanged {
                account: WORK,
                state: ConnectionState::Online,
            },
            Some(WORK),
        );
        trackers.apply(
            &Event::ConnectionChanged {
                account: HOME,
                state: ConnectionState::Offline,
            },
            Some(WORK),
        );

        assert_eq!(
            trackers.status(WORK).state,
            ConnectionState::Online,
            "Home going offline said nothing about Work"
        );
        assert_eq!(trackers.status(HOME).state, ConnectionState::Offline);
    }

    #[test]
    fn an_account_nothing_has_been_heard_about_is_working_locally() {
        let trackers = Trackers::default();
        assert_eq!(trackers.status(WORK).state, ConnectionState::Offline);
    }

    #[test]
    fn progress_lands_on_the_account_it_names_and_no_other() {
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::SyncProgress {
                account: HOME,
                done: 3,
                total: 10,
            },
            Some(WORK),
        );
        assert_eq!(trackers.status(HOME).progress, Some((3, 10)));
        assert_eq!(
            trackers.status(WORK).progress,
            None,
            "Work is not syncing and its line must not say it is"
        );
    }

    #[test]
    fn an_error_carries_no_account_so_it_lands_on_the_one_in_view() {
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::Error {
                message: "the server refused the password".to_owned(),
                account: None,
            },
            Some(WORK),
        );
        trackers.apply(
            &Event::ConnectionChanged {
                account: WORK,
                state: ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
            },
            Some(WORK),
        );
        assert_eq!(
            trackers.status(WORK).detail.as_deref(),
            Some("the server refused the password")
        );
        assert_eq!(
            trackers.status(HOME).detail,
            None,
            "an error with no account named must not be attributed to one"
        );
    }

    #[test]
    fn an_error_that_names_its_account_lands_there_whatever_is_in_view() {
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::Error {
                message: "mailbox is over quota".to_owned(),
                account: Some(HOME),
            },
            Some(WORK),
        );
        trackers.apply(
            &Event::ConnectionChanged {
                account: HOME,
                state: ConnectionState::Failing {
                    reason: postio_core::FailureReason::Server,
                },
            },
            Some(WORK),
        );
        assert_eq!(
            trackers.status(HOME).detail.as_deref(),
            Some("mailbox is over quota")
        );
        assert_eq!(trackers.status(WORK).detail, None);
    }

    #[test]
    fn statuses_are_reported_for_the_accounts_asked_for_in_that_order() {
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::ConnectionChanged {
                account: HOME,
                state: ConnectionState::Offline,
            },
            Some(WORK),
        );
        let named = trackers.statuses(&[WORK, HOME]);
        assert_eq!(named.len(), 2);
        assert_eq!(named[0].0, WORK);
        assert_eq!(named[0].1.state, ConnectionState::Offline, "never heard of");
        assert_eq!(named[1].0, HOME);
        assert_eq!(named[1].1.state, ConnectionState::Offline);
    }
}
