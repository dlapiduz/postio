//! Noticing that mail arrived: `IDLE` on one mailbox, interval polling on the
//! rest.
//!
//! # The failure mode this module is shaped around
//!
//! `IDLE` fails silently. A server that drops a connection it has not heard
//! from, a middle-box that reaps an idle NAT entry, a server that accepts the
//! command and then never sends an untagged response — all three look exactly
//! like a quiet mailbox. There is no error, no disconnect and nothing in a log.
//! The user's experience is "new mail stopped appearing", hours later, with no
//! way to tell that anything is wrong.
//!
//! So this module never trusts `IDLE` as the only evidence:
//!
//! * The command is re-issued well inside [`WatchPolicy::idle_refresh`], which
//!   is minutes rather than the twenty-nine RFC 2177 §3 permits — the cap is
//!   what the *protocol* tolerates, not what the path between two machines
//!   does.
//! * A mailbox that has only been idled on for [`WatchPolicy::poll_interval`]
//!   is reconciled with a plain `STATUS` regardless. That is the floor under a
//!   deaf connection: however badly push is behaving, new mail surfaces within
//!   one poll interval.
//! * Whether to idle at all is decided from the post-authentication
//!   [`Capabilities`], never from the untagged `* ENABLED` echo — some servers
//!   never send one (ADR 0001, "iCloud-specific hazards"), and a client that
//!   waits for it either never idles or, worse, believes it is idling.
//!
//! # Two lanes, because `IDLE` occupies a connection
//!
//! A connection inside `IDLE` cannot carry anything else, which is why the
//! watched mailbox gets its own. That is a real structural constraint rather
//! than an implementation detail, so it is in the API: [`Watcher::next_push`]
//! is what the dedicated connection should do, [`Watcher::next_poll`] what the
//! shared one should do, and the two never hand out the same mailbox.
//!
//! # No timers, no tasks, no connections
//!
//! Like [`Supervisor`](crate::connect::Supervisor), this is a state machine the
//! caller drives: it says what to do next and is told what happened, and the
//! layer that owns a runtime does the waiting. That is what keeps the whole
//! engine testable at an exact instant with no clock, and it is also what makes
//! the duplicate-connection guarantee checkable — see [`Watcher::next_push`].
//!
//! # What a wake-up means
//!
//! Nothing here interprets an event. [`MailboxEvent`] says *that* a mailbox
//! changed, never what it now holds, so the only correct response to any of
//! them is a resync pull ([`crate::resync::resync_mailbox`]). Applying them as
//! a diff is how a client and a server quietly stop agreeing.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_imap::backend::{Capabilities, Capability, MailboxEvent, MailboxStatus};
use postio_imap::cancel::CancelToken;
use postio_model::{MailboxId, ModSeq, Uid, UidValidity};

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// The longest RFC 2177 §3 lets a client hold one `IDLE` before re-issuing it.
///
/// A ceiling, not a target: [`WatchPolicy::idle_refresh`] is far below it
/// because the protocol's tolerance is not the network's.
pub const RFC2177_MAX_IDLE: Duration = Duration::from_secs(29 * 60);

/// How hard to watch, and how often to distrust the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchPolicy {
    /// Whether to use `IDLE` at all — `[sync] idle` in `config.toml`.
    ///
    /// Turning it off costs push delivery and buys one connection back, which
    /// is a trade some accounts and some servers want.
    pub idle: bool,
    /// How long one `IDLE` is held before it is re-issued.
    pub idle_refresh: Duration,
    /// How often a mailbox is reconciled with a `STATUS` — `[sync]
    /// poll_interval_secs`.
    ///
    /// Applies to every mailbox, the watched one included: see the module docs
    /// on why push is never the only evidence.
    pub poll_interval: Duration,
}

impl Default for WatchPolicy {
    /// Idle, re-arming every nine minutes, reconciling every five.
    ///
    /// Nine minutes is chosen against the network rather than the protocol.
    /// RFC 2177 would allow twenty-nine, and at the other extreme `io-imap`'s
    /// own watch loop re-arms every twenty-nine *seconds* — roughly 120 round
    /// trips an hour per mailbox, which is more than a server needs and more
    /// than a laptop on battery wants. Nine minutes is under the idle timeout
    /// of every consumer NAT and HTTP-proxy path we know of, and costs seven
    /// round trips an hour.
    fn default() -> Self {
        Self {
            idle: true,
            idle_refresh: Duration::from_secs(9 * 60),
            poll_interval: Duration::from_secs(300),
        }
    }
}

impl WatchPolicy {
    /// The `IDLE` duration to ask for, clamped to what RFC 2177 §3 permits.
    ///
    /// A misconfigured hour-long refresh would leave the client believing it is
    /// watching a mailbox the server stopped talking to fifty minutes ago.
    pub fn idle_timeout(&self) -> Duration {
        self.idle_refresh.min(RFC2177_MAX_IDLE)
    }
}

// ---------------------------------------------------------------------------
// What to do, and what came back
// ---------------------------------------------------------------------------

/// Whether a mailbox is watched by push or by polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// The one mailbox worth a connection of its own — in practice the inbox.
    ///
    /// Registering a second one replaces the first: there is one dedicated
    /// connection, and pretending otherwise would silently stop watching
    /// whichever mailbox lost.
    Push,
    /// Checked on [`WatchPolicy::poll_interval`] over the shared connection.
    Poll,
}

/// The next thing a connection should do.
#[derive(Debug)]
pub enum Watch {
    /// Hold an `IDLE` on `path` for `timeout`, then report through
    /// [`Watcher::woke`].
    ///
    /// `cancel` is the watcher's copy as well as the caller's:
    /// [`Watcher::suspend`] fires it, which is how a machine going to sleep
    /// closes the connection instead of leaking it.
    Idle {
        /// The mailbox being watched.
        mailbox: MailboxId,
        /// Its path on the server.
        path: String,
        /// How long to hold the command.
        timeout: Duration,
        /// Fired when the watcher wants the command to end early.
        cancel: CancelToken,
    },
    /// Ask the server for `path`'s state, and report through
    /// [`Watcher::observed`].
    Poll {
        /// The mailbox to check.
        mailbox: MailboxId,
        /// Its path on the server.
        path: String,
    },
    /// Nothing is due on this lane.
    ///
    /// `until` is when to come back; `None` means nothing will become due
    /// without something else happening first — the watcher is suspended, has
    /// nothing registered on this lane, or is waiting for a step it already
    /// handed out to be reported.
    Wait {
        /// When the next step falls due, if it can be known now.
        until: Option<DateTime<Utc>>,
    },
}

/// What a completed step means for the mailbox it was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Something moved. Pull, rather than trying to infer what.
    Changed,
    /// Nothing moved. Carry on watching.
    Quiet,
}

impl Wake {
    /// Whether the caller should resync this mailbox now.
    pub fn needs_resync(self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// The parts of a [`MailboxStatus`] that mean "something happened".
///
/// Compared instead of the whole struct so that a field describing the
/// *session* rather than the mailbox — `read_only`, the permanent flag list —
/// cannot make an unchanged folder look busy, and so that adding a field to
/// [`MailboxStatus`] cannot silently change what counts as a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    uid_validity: UidValidity,
    uid_next: Uid,
    exists: u32,
    unseen: Option<u32>,
    highest_mod_seq: Option<ModSeq>,
}

impl Signature {
    fn of(status: &MailboxStatus) -> Self {
        Self {
            uid_validity: status.uid_validity,
            uid_next: status.uid_next,
            exists: status.exists,
            unseen: status.unseen,
            highest_mod_seq: status.highest_mod_seq,
        }
    }
}

// ---------------------------------------------------------------------------
// The watcher
// ---------------------------------------------------------------------------

/// One mailbox's place in the rotation.
#[derive(Debug)]
struct Target {
    path: String,
    attention: Attention,
    /// When this mailbox may next be acted on.
    due_at: DateTime<Utc>,
    /// When the server last actually told us this mailbox's state — a `STATUS`
    /// answer, or an `IDLE` that reported something. A quiet `IDLE` is not
    /// evidence, which is the whole point of tracking this separately.
    verified_at: Option<DateTime<Utc>>,
    /// What the last verification said.
    signature: Option<Signature>,
    /// Set while a step is outstanding. Cleared only by a report.
    in_flight: Option<CancelToken>,
}

impl Target {
    /// Whether the mailbox is overdue a `STATUS` reconciliation.
    fn wants_verifying(&self, now: DateTime<Utc>, interval: Duration) -> bool {
        match self.verified_at {
            // Never looked at: we have only just connected and have no idea
            // what we missed while we were away.
            None => true,
            Some(at) => now - at >= delta(interval),
        }
    }
}

/// Decides what to watch, when, and on which connection.
///
/// Holds no connection and starts no task. See the module docs.
#[derive(Debug)]
pub struct Watcher {
    policy: WatchPolicy,
    /// Whether `IDLE` is both configured and advertised. Read from the
    /// post-authentication capability list, never from an `* ENABLED` echo.
    can_idle: bool,
    targets: HashMap<MailboxId, Target>,
    /// The one mailbox on the dedicated connection, when there is one.
    pushed: Option<MailboxId>,
    suspended: bool,
}

impl Watcher {
    /// A watcher for a server with `capabilities`, watching nothing yet.
    pub fn new(policy: WatchPolicy, capabilities: &Capabilities) -> Self {
        Self {
            policy,
            can_idle: policy.idle && capabilities.contains(Capability::Idle),
            targets: HashMap::new(),
            pushed: None,
            suspended: false,
        }
    }

    /// Re-reads the capability list, as a reconnection produces.
    ///
    /// A server may come back with less than it had — a failover to a node
    /// without `IDLE` is a real thing — and continuing to idle at it would be
    /// watching a mailbox nobody is listening to.
    pub fn set_capabilities(&mut self, capabilities: &Capabilities) {
        self.can_idle = self.policy.idle && capabilities.contains(Capability::Idle);
    }

    /// The policy in force.
    pub fn policy(&self) -> WatchPolicy {
        self.policy
    }

    /// Whether this server will actually be idled on.
    pub fn idles(&self) -> bool {
        self.can_idle
    }

    /// Whether the watcher is parked.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Registers a mailbox, or changes how an already-registered one is
    /// watched.
    ///
    /// Registering a second [`Attention::Push`] mailbox demotes the first to
    /// polling rather than silently dropping it: there is one dedicated
    /// connection, and a mailbox nobody watches at all is worse than one
    /// watched slowly.
    pub fn watch(&mut self, mailbox: MailboxId, path: impl Into<String>, attention: Attention) {
        let path = path.into();
        if attention == Attention::Push {
            if let Some(previous) = self.pushed.replace(mailbox)
                && previous != mailbox
                && let Some(target) = self.targets.get_mut(&previous)
            {
                target.attention = Attention::Poll;
            }
        } else if self.pushed == Some(mailbox) {
            self.pushed = None;
        }

        let target = self.targets.entry(mailbox).or_insert_with(|| Target {
            path: path.clone(),
            attention,
            due_at: DateTime::<Utc>::MIN_UTC,
            verified_at: None,
            signature: None,
            in_flight: None,
        });
        target.path = path;
        target.attention = attention;
    }

    /// Stops watching a mailbox — it was deleted, or unsubscribed.
    pub fn forget(&mut self, mailbox: MailboxId) {
        if let Some(target) = self.targets.remove(&mailbox)
            && let Some(cancel) = target.in_flight
        {
            cancel.cancel();
        }
        if self.pushed == Some(mailbox) {
            self.pushed = None;
        }
    }

    /// What the dedicated connection should do next.
    ///
    /// Returns a step at most once per mailbox until that step is reported
    /// through [`woke`](Self::woke), [`observed`](Self::observed) or
    /// [`failed`](Self::failed). That is the duplicate-connection guarantee:
    /// a caller cannot obtain two `IDLE`s for one mailbox by asking twice,
    /// however confused its own task bookkeeping gets.
    pub fn next_push(&mut self, now: DateTime<Utc>) -> Watch {
        let Some(mailbox) = self.pushed else {
            return Watch::Wait { until: None };
        };
        self.step(mailbox, now)
    }

    /// What the shared connection should do next.
    ///
    /// The mailbox that is due soonest, never the pushed one — that belongs to
    /// [`next_push`](Self::next_push), and checking it from here would be the
    /// duplicate work `IDLE` exists to avoid.
    pub fn next_poll(&mut self, now: DateTime<Utc>) -> Watch {
        if self.suspended {
            return Watch::Wait { until: None };
        }

        let mut due: Option<MailboxId> = None;
        let mut next: Option<DateTime<Utc>> = None;
        for (id, target) in &self.targets {
            if self.pushed == Some(*id) {
                continue;
            }
            if target.in_flight.is_some() {
                continue;
            }
            if now >= target.due_at {
                // Ties broken by id so the rotation is deterministic rather
                // than at the mercy of hash order.
                if due.is_none_or(|current| {
                    (target.due_at, *id) < (self.targets[&current].due_at, current)
                }) {
                    due = Some(*id);
                }
            } else if next.is_none_or(|at| target.due_at < at) {
                next = Some(target.due_at);
            }
        }

        match due {
            Some(mailbox) => self.step(mailbox, now),
            None => Watch::Wait { until: next },
        }
    }

    /// Reports what an `IDLE` returned.
    ///
    /// Any event at all is [`Wake::Changed`]: the events say only *that* the
    /// mailbox moved. An empty slice is the ordinary quiet expiry, and also
    /// what a cancelled `IDLE` reports.
    pub fn woke(
        &mut self,
        mailbox: MailboxId,
        events: &[MailboxEvent],
        now: DateTime<Utc>,
    ) -> Wake {
        let Some(target) = self.targets.get_mut(&mailbox) else {
            return Wake::Quiet;
        };
        target.in_flight = None;
        // Re-arm immediately: an idle that has just returned is a connection
        // sitting silent, and silence is what gets it dropped.
        target.due_at = now;

        if events.is_empty() {
            return Wake::Quiet;
        }
        // The pull the caller is about to run is itself a verification, so a
        // busy mailbox does not also pay for a STATUS.
        target.verified_at = Some(now);
        target.signature = None;
        Wake::Changed
    }

    /// Reports what a `STATUS` said.
    ///
    /// The first look at a mailbox is always [`Wake::Changed`]: we have just
    /// connected, and nothing local can say what happened while we were gone.
    pub fn observed(
        &mut self,
        mailbox: MailboxId,
        status: &MailboxStatus,
        now: DateTime<Utc>,
    ) -> Wake {
        let idled = self.can_idle && self.pushed == Some(mailbox);
        let interval = self.policy.poll_interval;
        let Some(target) = self.targets.get_mut(&mailbox) else {
            return Wake::Quiet;
        };

        let signature = Signature::of(status);
        let changed = target.signature != Some(signature);

        target.in_flight = None;
        target.verified_at = Some(now);
        target.signature = Some(signature);
        // A mailbox that is about to be idled on goes straight back to it;
        // everything else waits out the interval.
        target.due_at = if idled { now } else { now + delta(interval) };

        if changed { Wake::Changed } else { Wake::Quiet }
    }

    /// Reports that a step failed.
    ///
    /// The mailbox is released rather than left in flight forever — a wedged
    /// target is a mailbox that silently stops being watched — and is held off
    /// for one poll interval, by which time
    /// [`Supervisor`](crate::connect::Supervisor) has had its say about whether
    /// the connection is coming back at all.
    pub fn failed(&mut self, mailbox: MailboxId, now: DateTime<Utc>) {
        let interval = self.policy.poll_interval;
        if let Some(target) = self.targets.get_mut(&mailbox) {
            target.in_flight = None;
            target.due_at = now + delta(interval);
        }
    }

    /// Parks the watcher and ends every outstanding command.
    ///
    /// What suspending a laptop means. Cancelling is the whole point: an
    /// `IDLE` left running holds a connection open on a machine that is about
    /// to stop answering, and the server keeps it — and its resources —
    /// until it times out.
    ///
    /// The cancelled steps stay *outstanding* until their callers report back,
    /// so nothing is handed out again while a command is still unwinding. That
    /// is what makes resume unable to open a second connection.
    pub fn suspend(&mut self) {
        self.suspended = true;
        for target in self.targets.values() {
            if let Some(cancel) = &target.in_flight {
                cancel.cancel();
            }
        }
    }

    /// Un-parks the watcher, and makes everything due at once.
    ///
    /// A machine that has just woken has no idea how long it was away, so every
    /// mailbox is checked immediately rather than waiting out an interval that
    /// was measured before the lid closed.
    pub fn resume(&mut self, now: DateTime<Utc>) {
        self.suspended = false;
        for target in self.targets.values_mut() {
            target.due_at = now;
            // Whatever we knew about the server, we knew it before an
            // indeterminate gap.
            target.verified_at = None;
        }
    }

    /// Hands out the step `mailbox` is due, marking it outstanding.
    fn step(&mut self, mailbox: MailboxId, now: DateTime<Utc>) -> Watch {
        if self.suspended {
            return Watch::Wait { until: None };
        }
        let can_idle = self.can_idle;
        let timeout = self.policy.idle_timeout();
        let interval = self.policy.poll_interval;
        let Some(target) = self.targets.get_mut(&mailbox) else {
            return Watch::Wait { until: None };
        };

        if target.in_flight.is_some() {
            return Watch::Wait { until: None };
        }
        if now < target.due_at {
            return Watch::Wait {
                until: Some(target.due_at),
            };
        }

        // The watched mailbox idles, except when it is overdue the `STATUS`
        // that is the floor under a connection which has gone deaf.
        if can_idle && self.pushed == Some(mailbox) && !target.wants_verifying(now, interval) {
            let cancel = CancelToken::new();
            target.in_flight = Some(cancel.clone());
            return Watch::Idle {
                mailbox,
                path: target.path.clone(),
                timeout,
                cancel,
            };
        }

        // A `STATUS` is a round trip and cannot be cancelled halfway to any
        // useful effect, but it still carries a token: it is what marks the
        // mailbox outstanding, and it is what a suspend fires at.
        target.in_flight = Some(CancelToken::new());
        Watch::Poll {
            mailbox,
            path: target.path.clone(),
        }
    }
}

/// A [`Duration`] as a [`TimeDelta`], saturating rather than failing on a
/// configured interval nobody could mean.
fn delta(duration: Duration) -> TimeDelta {
    TimeDelta::from_std(duration).unwrap_or(TimeDelta::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(names: &[&str]) -> Capabilities {
        Capabilities::from_names(names.iter().copied())
    }

    fn idling() -> Capabilities {
        capabilities(&["IMAP4rev1", "IDLE", "CONDSTORE", "QRESYNC"])
    }

    #[test]
    fn the_default_refresh_is_far_under_what_the_rfc_permits() {
        let policy = WatchPolicy::default();

        assert!(
            policy.idle_refresh < RFC2177_MAX_IDLE,
            "the RFC's cap is what the protocol tolerates, not what a NAT does"
        );
        assert_eq!(policy.idle_timeout(), policy.idle_refresh);
    }

    #[test]
    fn an_over_long_refresh_is_clamped_to_the_rfc_ceiling() {
        let policy = WatchPolicy {
            idle_refresh: Duration::from_secs(3_600),
            ..WatchPolicy::default()
        };

        assert_eq!(
            policy.idle_timeout(),
            RFC2177_MAX_IDLE,
            "an hour-long IDLE leaves the client watching a mailbox the server \
             stopped talking to fifty minutes ago"
        );
    }

    #[test]
    fn idle_is_gated_on_the_capability_list_and_on_configuration() {
        let policy = WatchPolicy::default();

        assert!(Watcher::new(policy, &idling()).idles());
        assert!(
            !Watcher::new(policy, &capabilities(&["IMAP4rev1"])).idles(),
            "a server that does not advertise IDLE must never be idled at, \
             whatever it echoed back from ENABLE"
        );
        assert!(
            !Watcher::new(
                WatchPolicy {
                    idle: false,
                    ..policy
                },
                &idling()
            )
            .idles()
        );
    }

    #[test]
    fn losing_idle_on_a_reconnect_is_noticed() {
        let mut watcher = Watcher::new(WatchPolicy::default(), &idling());
        assert!(watcher.idles());

        // A failover to a node with a smaller feature set.
        watcher.set_capabilities(&capabilities(&["IMAP4rev1"]));

        assert!(
            !watcher.idles(),
            "continuing to idle at a server that cannot is watching a mailbox \
             nobody is listening to"
        );
    }

    #[test]
    fn a_second_pushed_mailbox_demotes_the_first_rather_than_dropping_it() {
        let mut watcher = Watcher::new(WatchPolicy::default(), &idling());
        let first = MailboxId::new(1);
        let second = MailboxId::new(2);

        watcher.watch(first, "INBOX", Attention::Push);
        watcher.watch(second, "Later", Attention::Push);

        assert_eq!(watcher.pushed, Some(second));
        assert_eq!(
            watcher.targets[&first].attention,
            Attention::Poll,
            "a mailbox nobody watches at all is worse than one watched slowly"
        );
    }

    #[test]
    fn forgetting_a_mailbox_ends_whatever_it_had_outstanding() {
        let mut watcher = Watcher::new(WatchPolicy::default(), &idling());
        let inbox = MailboxId::new(1);
        watcher.watch(inbox, "INBOX", Attention::Push);
        watcher.next_push(DateTime::<Utc>::MIN_UTC);
        let outstanding = watcher.targets[&inbox]
            .in_flight
            .clone()
            .expect("a step is outstanding");

        watcher.forget(inbox);

        assert!(outstanding.is_cancelled());
        assert_eq!(watcher.pushed, None);
    }

    #[test]
    fn a_watcher_with_nothing_registered_asks_for_nothing() {
        let mut watcher = Watcher::new(WatchPolicy::default(), &idling());
        let now = DateTime::<Utc>::MIN_UTC;

        assert!(matches!(
            watcher.next_push(now),
            Watch::Wait { until: None }
        ));
        assert!(matches!(
            watcher.next_poll(now),
            Watch::Wait { until: None }
        ));
    }
}
