//! The sync status line's words: what the sidebar's foot says about the
//! connection, and the input the list pane's state machine reads.
//!
//! [`SyncStatus`] is assembled from `ConnectionChanged` and `SyncProgress`
//! on the core event stream and answered in words — `idle · imap`,
//! `fetched 1204`, `last sync 12s` — by [`SyncStatus::lines`]. It lives
//! here, toolkit-free, because two surfaces read it: the sidebar's status
//! line draws it, and [`crate::list_state`] derives the list pane's plates
//! from it. One implementation, called by both frontends, so the macOS
//! status line and the GTK one say the same thing about the same sync.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use postio_core::{ConnectionState, Event};
use postio_model::AccountId;
use postio_model::mailbox::Mailbox;

/// The protocol the status line names. v1 is IMAP only (CLAUDE.md).
const PROTOCOL: &str = "imap";

/// What the status line has to say.
///
/// Assembled from `ConnectionChanged` and `SyncProgress` on the core event
/// stream. `last_sync` is an [`Instant`] rather than a wall-clock time because
/// the line shows an age, and an age must not jump when the system clock is
/// corrected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStatus {
    /// Where the account stands with its server.
    pub state: ConnectionState,
    /// When the last sync completed.
    pub last_sync: Option<Instant>,
    /// Completed and expected units of a long resync.
    pub progress: Option<(u32, u32)>,
    /// Settled and queued mail, while a backfill is running.
    ///
    /// Kept apart from [`progress`](Self::progress) rather than folded into
    /// it, because the two phases mean different things to someone looking
    /// at the sidebar: a list still arriving cannot be read, and mail whose
    /// text is still arriving can. See issue #74.
    pub backfill: Option<(u32, u32)>,
    /// Why the connection is failing, phrased for the user.
    pub detail: Option<String>,
    /// What this account's mail weighs, and how much is already here.
    ///
    /// `None` until something has measured it. Carried on the status rather
    /// than fetched by the line, because it arrives on
    /// [`Event::BackfillProgress`] alongside the counts it is drawn beside
    /// (#411, ADR 0017).
    ///
    /// [`Event::BackfillProgress`]: postio_core::Event::BackfillProgress
    pub footprint: Option<postio_core::event::MailFootprint>,
}

impl Default for SyncStatus {
    /// What Postio shows before it has ever reached a server.
    fn default() -> Self {
        SyncStatus {
            state: ConnectionState::Offline,
            last_sync: None,
            progress: None,
            backfill: None,
            detail: None,
            footprint: None,
        }
    }
}

impl SyncStatus {
    /// The two lines the canvas draws, as of `now`.
    pub fn lines(&self, now: Instant) -> (String, String) {
        (
            format!("{} · {PROTOCOL}", self.state_word()),
            self.detail_line(now),
        )
    }

    /// The detail line with the byte clause the column cannot hold, for the
    /// tooltip and the accessible description.
    ///
    /// The sidebar is 212px by canvas 1b and deliberately fixed, which is
    /// about 25 monospace characters; `mail 12400 of 81744` is already 19.
    /// So on that line it is counts or bytes, never both, and #411 settled
    /// which: a count that climbs answers *"is anything happening"*, which
    /// is what #74 filed this line for, and a byte figure that sits still
    /// through a large fetch reads as stalled. Bytes are a cost signal, and
    /// cost is asked once and deliberately.
    ///
    /// They still reach this surface, just not 25 columns of it. A screen
    /// reader and a hover both get the number, and both get it from here, so
    /// the two cannot drift.
    pub fn detail_in_full(&self, now: Instant) -> String {
        let detail = self.detail_line(now);
        // Only while a backfill is running: anywhere else there is no count
        // for the bytes to be a second clause of.
        match self.filling().and(self.bytes_clause()) {
            Some(bytes) => format!("{detail} · {bytes}"),
            None => detail,
        }
    }

    fn state_word(&self) -> String {
        match self.state {
            ConnectionState::Offline => "offline".to_string(),
            ConnectionState::Connecting => "connecting".to_string(),
            ConnectionState::Failing { .. } => "error".to_string(),
            ConnectionState::Online if self.syncing().is_some() => "syncing".to_string(),
            // The list is complete and the mail itself is not. Its own word,
            // because "syncing" already means the list and "idle" was the
            // lie issue #74 was filed about. It matches what the reading
            // pane says about a message it has no body for, which is the
            // same fact seen from the other end.
            ConnectionState::Online if self.filling().is_some() => "downloading".to_string(),
            ConnectionState::Online => "idle".to_string(),
        }
    }

    /// How many messages the pass that is running has fetched, if one is.
    ///
    /// One question, asked once, and both lines answer from it — which is the
    /// whole of `postio-qhz.6`. The first live sync said "0% synced" and
    /// "never synced" together because the two lines were reading different
    /// sources: progress from `SyncProgress`, "never synced" from a
    /// `last_synced_at` that only moves when a pass *completes*. Neither was
    /// wrong on its own terms and the pair was useless.
    ///
    /// `progress` is `Some` exactly while a pass is in flight — `SyncTracker`
    /// clears it on any connection change and when `done` reaches `total` —
    /// so its presence is the answer to "is anything happening".
    fn syncing(&self) -> Option<u32> {
        match self.progress {
            // A pass with nothing to reach never started.
            Some((_, 0)) => None,
            Some((done, total)) if done < total => Some(done),
            _ => None,
        }
    }

    /// How much mail the backfill has settled, if a backfill is running.
    ///
    /// `None` once the queue has drained, so a finished backfill falls back
    /// to the ordinary idle line rather than sticking at `2000 of 2000` —
    /// the same trap `syncing` fell into and the same answer.
    fn filling(&self) -> Option<(u32, u32)> {
        match self.backfill {
            // A queue with nothing in it is not a backfill in progress.
            Some((_, 0)) => None,
            Some((done, total)) if done < total => Some((done, total)),
            _ => None,
        }
    }

    /// `890 MB of 1.4 GB`, when there is a measured size worth claiming.
    ///
    /// Feeds [`detail_in_full`](Self::detail_in_full) only — the drawn line
    /// has no room for it (#411).
    ///
    /// `None` in the two cases where a size would be a lie rather than a
    /// number:
    ///
    /// * **nothing measured yet** — no footprint has arrived;
    /// * **an empty account** — `0 B of 0 B` reads as a bug, not as "no mail".
    ///   An account with nothing in it owes no size claim at all.
    ///
    /// While the header pass is still running every figure is a lower bound,
    /// so the total is written `over 1.4 GB`. Only the total carries the
    /// hedge: what is already downloaded is known exactly, and hedging it too
    /// would say the local figure might grow for a different reason than it
    /// will.
    fn bytes_clause(&self) -> Option<String> {
        let footprint = self.footprint.as_ref()?;
        if footprint.total_bytes == 0 {
            return None;
        }
        Some(format!(
            "{} of {}",
            crate::format::human_size(footprint.local_bytes),
            crate::format::human_size_bound(footprint.total_bytes, footprint.complete),
        ))
    }

    /// The second line: the reason it is failing, or how long ago it worked.
    ///
    /// The reason wins. "last sync 4h" is not what someone needs to read when
    /// the password has expired.
    fn detail_line(&self, now: Instant) -> String {
        if matches!(self.state, ConnectionState::Failing { .. })
            && let Some(detail) = &self.detail
        {
            return detail.clone();
        }
        // A pass that is running says what it has, not when it last finished
        // and not a percentage. The denominator is `UIDNEXT - 1` — the highest
        // UID the pass *could* reach, which expunged messages leave gaps in —
        // so a pass routinely finishes well short of it and a percentage of it
        // is a number that does not mean what it looks like. A count that
        // climbs answers "is anything happening", which is the only question
        // this line is being asked during a first sync.
        //
        // No thousands separator: the folder counts beside it are written
        // `4291`, and two number formats in one column read as two kinds of
        // number.
        if let Some(fetched) = self.syncing() {
            return format!("fetched {fetched}");
        }
        // Unlike the list pass, a backfill knows its real denominator: every
        // message that has entered the queue is in exactly one of the counts
        // `BackfillProgress` keeps. So this one can honestly say "of", which
        // "fetched 1204" above deliberately cannot.
        if let Some((done, total)) = self.filling() {
            return format!("mail {done} of {total}");
        }
        match self.last_sync {
            Some(at) => format!("last sync {}", age(now.saturating_duration_since(at))),
            None => "never synced".to_string(),
        }
    }

    /// How long until the age on the second line would read differently.
    ///
    /// `None` when nothing is ticking. The point is to not wake the process up
    /// once a second forever: seconds only matter while the answer is in
    /// seconds.
    pub fn refresh_interval(&self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.last_sync?);
        Some(match elapsed.as_secs() {
            ..60 => Duration::from_secs(1),
            60..3600 => Duration::from_secs(30),
            _ => Duration::from_secs(300),
        })
    }
}

/// A duration in the canvas' compact form: `12s`, `4m`, `3h`, `2d`.
pub fn age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

// ── Folding events into the line ─────────────────────────────────────────
//
// Moved from `postio-gtk::feed` for the terminal frontend, which folds the
// same events into the same line (specs/005-tui-frontend FR-004): logic two
// frontends need is written once.

/// The status line, folded out of the runtime's events.
///
/// Pure, and separate from the widget, because "what does the status line
/// say when the connection drops mid-resync" is a question worth answering
/// without a display in the loop.
///
/// # Where the failure reason comes from
///
/// [`ConnectionState::Failing`] carries a typed category — what *kind* of
/// help the account needs (ADR 0005 Q10) — but not prose. The prose travels
/// beside it as [`Event::Error`], so the tracker keeps the last one it saw
/// and promotes it the moment the connection starts failing. Leaving that
/// state clears it: a reason that outlived the failure it explained would be
/// worse than none.
#[derive(Clone, Debug, Default)]
pub struct SyncTracker {
    status: SyncStatus,
    /// The last error seen, waiting to explain a failure that may not come.
    reason: Option<String>,
}

/// One [`SyncTracker`] per account, so no account's server speaks for another.
///
/// Every status-bearing event names the account it is about, and a single
/// tracker threw that away: with two accounts configured, the status line
/// showed whichever server reported most recently. That is invisible with one
/// account, which is why it survived — and it is load-bearing for ADR 0005
/// Q10, whose whole subject is *which* account is not answering.
///
/// [`Event::Error`] is the exception, because it carries no account. It goes
/// to the account whose line is on screen, which is exactly what the single
/// tracker did with it; writing it down here makes it a decision rather than
/// an accident of which arm ran first.
#[derive(Clone, Debug, Default)]
pub struct Trackers {
    per_account: std::collections::BTreeMap<AccountId, SyncTracker>,
}

impl Trackers {
    /// Fold `event` in, routed to the account it names.
    ///
    /// `current` is the account whose status line is on screen, used only
    /// for the events that name none. Returns whether anything changed.
    pub fn apply(&mut self, event: &Event, current: Option<AccountId>) -> bool {
        let account = match event {
            Event::ConnectionChanged { account, .. }
            | Event::SyncProgress { account, .. }
            | Event::BackfillProgress { account, .. } => Some(*account),
            _ => current,
        };
        let Some(account) = account else {
            return false;
        };
        self.per_account.entry(account).or_default().apply(event)
    }

    /// What `account`'s line should say.
    ///
    /// An account nothing has been heard about is offline — the same default
    /// [`postio_core::AppState::connection`] gives, and for the same reason:
    /// silence is not a claim that the server is reachable.
    pub fn status(&self, account: AccountId) -> SyncStatus {
        self.per_account
            .get(&account)
            .map(|tracker| tracker.status().clone())
            .unwrap_or_default()
    }

    /// Fold `account`'s own folders' last-sync time into its tracker.
    ///
    /// Per account and not over the whole flat list: in section mode the
    /// sidebar reads every account's tree into one vector, and the newest
    /// `last_synced_at` in it belongs to whichever account synced most
    /// recently — which is exactly the cross-account confusion this type
    /// exists to end.
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

    /// The statuses of `accounts`, in the order given.
    ///
    /// The caller's order, because it is the sidebar's, which is the order
    /// the per-account hues are keyed to. An account nothing has been heard
    /// about still gets an entry: dropping it would be its own omission, in
    /// the one place whose subject is not omitting things.
    pub fn statuses(&self, accounts: &[AccountId]) -> Vec<(AccountId, SyncStatus)> {
        accounts
            .iter()
            .map(|account| (*account, self.status(*account)))
            .collect()
    }
}

impl SyncTracker {
    /// A tracker that has heard nothing yet: offline, never synced.
    pub fn new() -> Self {
        Self::default()
    }

    /// What the status line should say.
    pub fn status(&self) -> &SyncStatus {
        &self.status
    }

    /// Fold `event` in. Returns whether the status line changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        let before = self.status.clone();
        match event {
            Event::ConnectionChanged { state, .. } => {
                self.status.state = *state;
                if matches!(state, ConnectionState::Failing { .. }) {
                    self.status.detail = self.reason.clone();
                } else {
                    // Connected, connecting or deliberately offline: whatever
                    // went wrong before is no longer what is happening.
                    self.status.detail = None;
                    self.reason = None;
                }
                // A pass's progress belongs to that pass. The engine announces
                // a connection state at the *boundaries* — a pass starting or
                // finishing, or the link itself moving — and never between two
                // batches, so any of them means the number on screen is no
                // longer being made.
                //
                // Including `Online`, which is the case that matters: a pass
                // ends by moving the tracker to idle, and idle is announced as
                // `Online`. `SyncProgress` only clears itself when `done`
                // reaches `total`, and `total` is `UIDNEXT - 1` — an upper
                // bound that expunged messages leave gaps in, so a pass can
                // finish having never reached it. Leaving `Online` alone left
                // the line reading `syncing 89%` on a folder that had finished,
                // for as long as the account stayed connected.
                self.status.progress = None;
                // The body queue's number is deliberately *not* cleared here
                // (issue #316). The reasoning above is true for a list pass,
                // which really does end at a connection boundary — but a
                // backfill does not: it spans many IDLE cycles and
                // reconnects while it keeps running, so `ConnectionChanged`
                // fires constantly in the middle of one. Dropping the count
                // on every one of those left the line reading `idle` for as
                // long as it took the *next* body to settle and the 250 ms
                // floor on top of that, while a body was genuinely still on
                // the wire. `BackfillProgress` clears the count itself once
                // the queue actually drains — that is the boundary that
                // matters for this number, not a connection event.
            }
            Event::BackfillProgress {
                done,
                total,
                footprint,
                ..
            } => {
                self.status.backfill = Some((*done, *total));
                // Kept even when the queue drains below: the size of an
                // account's mail is true whether or not a backfill is
                // running, and the settings panel asks for it at a moment
                // that has nothing to do with one.
                if footprint.is_some() {
                    self.status.footprint = *footprint;
                }
                // Drained. Clear it rather than leaving `2000 of 2000` on
                // screen -- the same trap `SyncProgress` documents above,
                // and the same answer. `last_sync` is deliberately not
                // touched: it means a *list* pass completed, and a body
                // queue draining is not that.
                if done >= total {
                    self.status.backfill = None;
                }
            }
            Event::SyncProgress { done, total, .. } => {
                self.status.progress = Some((*done, *total));
                // A resync that reached its own total is a sync that
                // finished, and that is when "last sync" moved.
                if done >= total {
                    self.status.last_sync = Some(Instant::now());
                    self.status.progress = None;
                }
            }
            Event::Error { message } => {
                self.reason = Some(message.clone());
                if matches!(self.status.state, ConnectionState::Failing { .. }) {
                    self.status.detail = Some(message.clone());
                }
            }
            _ => return false,
        }
        self.status != before
    }

    /// Record when this account last completed a sync, from its folders.
    ///
    /// [`SyncStatus::last_sync`] is an [`Instant`] on purpose: the line shows
    /// an *age*, and an age that jumped when the system clock was corrected
    /// would be worse than no age at all. The stored time is wall-clock, so
    /// the conversion happens here, once, at the boundary.
    pub fn note_last_sync(&mut self, mailboxes: &[Mailbox]) -> bool {
        let Some(latest) = mailboxes.iter().filter_map(|m| m.last_synced_at).max() else {
            return false;
        };
        let converted = to_instant(latest, Utc::now(), Instant::now());
        if converted.is_some() && self.status.last_sync.is_none() {
            self.status.last_sync = converted;
            return true;
        }
        false
    }
}

/// A wall-clock time as a point on the monotonic clock, relative to `now`.
///
/// `None` for a time in the future or further back than the process has been
/// running: neither can be expressed as an `Instant`, and inventing one would
/// put a fabricated age on the status line.
pub fn to_instant(at: DateTime<Utc>, now: DateTime<Utc>, monotonic: Instant) -> Option<Instant> {
    let age = now.signed_duration_since(at).to_std().ok()?;
    monotonic.checked_sub(age)
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use postio_model::MailboxId;
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

    /// Issue #74: the backfill's progress reached nobody, so the longest
    /// phase of a first sync drew `idle`.
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
                // Nothing measured yet: these predate the field, and they are
                // about the counter, not the size.
                footprint: None,
            }),
            "the status changed and the tracker said it had not"
        );
        assert_eq!(tracker.status().backfill, Some((412, 2000)));

        // Drained. It must clear itself the way `SyncProgress` does, or the
        // line reads `downloading` for as long as the account stays up.
        tracker.apply(&Event::BackfillProgress {
            account: AccountId::new(1),
            done: 2000,
            total: 2000,
            // Nothing measured yet: these predate the field, and they are
            // about the counter, not the size.
            footprint: None,
        });
        assert_eq!(
            tracker.status().backfill,
            None,
            "a queue that has drained is not a backfill in progress"
        );
        assert_eq!(tracker.status().lines(Instant::now()).0, "idle · imap");
    }

    /// Issue #316: `ConnectionChanged` cleared `backfill` unconditionally, on
    /// the reasoning that "the engine announces a connection state at the
    /// boundaries" — true for a list pass, which really does end there, but
    /// not for a backfill, which spans many IDLE cycles and reconnects while
    /// it keeps running. A connection blip mid-backfill erased the count the
    /// line was showing and the sidebar read `idle` while a body was still
    /// on the wire — seen live at the same moment the reading pane showed
    /// "Downloading this message" for the selected message.
    #[test]
    fn a_connection_announcement_mid_backfill_does_not_erase_its_count() {
        let mut tracker = SyncTracker::new();
        tracker.apply(&connection(ConnectionState::Online));
        tracker.apply(&Event::BackfillProgress {
            account: account(),
            done: 412,
            total: 2000,
            // Nothing measured yet: these predate the field, and they are
            // about the counter, not the size.
            footprint: None,
        });
        assert_eq!(
            tracker.status().lines(Instant::now()).0,
            "downloading · imap"
        );

        // The engine announces an IDLE cycle or a reconnect mid-backfill the
        // same way it announces anything else on the link: a connection
        // state, here `Online` again rather than a drain.
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
        // `last_sync` is what "last sync 4h" reads, and it means a *list*
        // pass completed. A backfill finishing is not that, and moving it
        // would date the mailbox from the wrong event.
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
            // Nothing measured yet: these predate the field, and they are
            // about the counter, not the size.
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

        // A resync that reaches its own total is a sync that finished.
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
        // The reason arrives beside the state change, not inside it.
        tracker.apply(&Event::Error {
            message: "the server rejected the password".to_string(),
        });
        tracker.apply(&connection(ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        }));
        assert_eq!(
            tracker.status().detail.as_deref(),
            Some("the server rejected the password")
        );

        // And an error that arrives while already failing replaces it.
        tracker.apply(&Event::Error {
            message: "the certificate expired".to_string(),
        });
        assert_eq!(
            tracker.status().detail.as_deref(),
            Some("the certificate expired")
        );

        // Recovering clears it: a reason that outlived its failure is worse
        // than no reason.
        tracker.apply(&connection(ConnectionState::Online));
        assert_eq!(tracker.status().detail, None);

        // And it does not come back on the next unrelated failure.
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
        // `total` is `UIDNEXT - 1`: an upper bound, not a promise, because
        // expunged messages leave gaps in the UID space. So a pass can finish
        // having fetched everything there is and still never reach it, and the
        // last report before it ended is a percentage below 100.
        //
        // The pass ending is announced as idle, which reaches the tracker as
        // `Online`. If that did not clear the number, the line would read
        // `syncing 89% · imap` for as long as the account stayed connected —
        // on a folder with nothing left to sync.
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
        }));
    }

    #[test]
    fn the_last_sync_age_comes_off_the_monotonic_clock() {
        let now = Utc::now();
        let monotonic = Instant::now();

        // An hour ago is an hour ago, whatever the wall clock does next.
        let hour = to_instant(now - chrono::Duration::hours(1), now, monotonic)
            .expect("an hour is expressible");
        assert!((monotonic.duration_since(hour).as_secs() as i64 - 3600).abs() <= 1);

        // A time in the future is not an age, and is refused rather than
        // turned into one.
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

        // The newest of them wins: one stale folder does not make the
        // account look stale.
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
        // The bug this type exists to fix: one tracker folded every
        // account's `ConnectionChanged` into one status, last writer wins,
        // so with two accounts the sidebar's line showed whichever server
        // happened to report most recently.
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
        // The same default `AppState::connection` gives, and for the same
        // reason: silence is not a claim that the server is reachable.
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
        // `Event::Error` has no account field. Routing it to the account
        // whose line is on screen is exactly what the single tracker did,
        // so this is no worse -- and it is written down here rather than
        // left as an accident of which arm ran.
        let mut trackers = Trackers::default();
        trackers.apply(
            &Event::Error {
                message: "the server refused the password".to_owned(),
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
    fn statuses_are_reported_for_the_accounts_asked_for_in_that_order() {
        // The order is the caller's -- the sidebar's -- because that is the
        // order the hues are keyed to, and an account absent from the map
        // still has to appear rather than silently drop out of the banner.
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
