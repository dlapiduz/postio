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

use postio_core::ConnectionState;

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
