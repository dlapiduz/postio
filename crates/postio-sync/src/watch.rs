//! Watches for new mail: `IDLE` on one mailbox, interval polling on the rest. `IDLE` fails
//! silently, so it is never the only evidence:
//! it re-arms well inside `idle_refresh`, and a `STATUS` every `poll_interval` is the floor under a
//! deaf connection. Idling is decided from post-auth `Capabilities`, never an `* ENABLED` echo (ADR
//! 0001).
//! A state machine the caller drives, with no timers or connections. A wake-up says only that a
//! mailbox changed; the response is always a resync pull.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use postio_account::backend::{Capabilities, Capability, MailboxEvent, MailboxStatus};
use postio_account::cancel::CancelToken;
use postio_model::{MailboxId, ModSeq, Uid};

/// A ceiling, not a target: `WatchPolicy::idle_refresh` is far below it because the protocol's
/// tolerance is not the network's.
pub const RFC2177_MAX_IDLE: Duration = Duration::from_secs(29 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchPolicy {
    pub idle: bool,
    pub idle_refresh: Duration,
    /// Applies to every mailbox, the watched one included: push is never the only evidence.
    pub poll_interval: Duration,
    /// `[sync] check_for_mail = "manual"` (#1013): never idle or poll on a timer; only an explicit
    /// check-for-mail reaches the server, through `Engine::sync`, not this watcher.
    pub manual: bool,
}

impl Default for WatchPolicy {
    /// Idle, re-arming every nine minutes (under every consumer NAT and proxy idle timeout),
    /// reconciling every five.
    fn default() -> Self {
        Self {
            idle: true,
            idle_refresh: Duration::from_secs(9 * 60),
            poll_interval: Duration::from_secs(300),
            manual: false,
        }
    }
}

impl WatchPolicy {
    /// Clamped to RFC 2177 §3: a misconfigured hour-long refresh would leave the client watching a
    /// mailbox the server stopped talking to.
    pub fn idle_timeout(&self) -> Duration {
        self.idle_refresh.min(RFC2177_MAX_IDLE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// The one mailbox with a connection of its own, in practice the inbox. A second registration
    /// replaces the first.
    Push,
    Poll,
}

#[derive(Debug)]
pub enum Watch {
    /// `cancel` is also the watcher's copy: `Watcher::suspend` fires it so a sleeping machine
    /// closes the connection instead of leaking it.
    Idle {
        mailbox: MailboxId,
        path: String,
        timeout: Duration,
        cancel: CancelToken,
    },
    Poll {
        mailbox: MailboxId,
        path: String,
    },
    /// `until` is when to come back; `None` means nothing is due without something else happening
    /// first (suspended, nothing registered, or a step awaiting its report).
    Wait {
        until: Option<DateTime<Utc>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    Changed,
    Quiet,
}

impl Wake {
    pub fn needs_resync(self) -> bool {
        matches!(self, Self::Changed)
    }
}

/// Compared instead of the whole `MailboxStatus` so session-level fields (`read_only`, permanent
/// flags) cannot make an unchanged folder look busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    generation: postio_model::Generation,
    uid_next: Uid,
    exists: u32,
    unseen: Option<u32>,
    highest_mod_seq: Option<ModSeq>,
}

impl Signature {
    fn of(status: &MailboxStatus) -> Self {
        Self {
            generation: status.generation,
            uid_next: status.uid_next,
            exists: status.exists,
            unseen: status.unseen,
            highest_mod_seq: status.highest_mod_seq,
        }
    }
}

#[derive(Debug)]
struct Target {
    path: String,
    attention: Attention,
    due_at: DateTime<Utc>,
    /// When the server last told us this mailbox's state (a `STATUS` answer, or an `IDLE` that
    /// reported); a quiet `IDLE` is not evidence.
    verified_at: Option<DateTime<Utc>>,
    signature: Option<Signature>,
    in_flight: Option<CancelToken>,
}

impl Target {
    fn wants_verifying(&self, now: DateTime<Utc>, interval: Duration) -> bool {
        match self.verified_at {
            None => true,
            Some(at) => now - at >= delta(interval),
        }
    }
}

#[derive(Debug)]
pub struct Watcher {
    policy: WatchPolicy,
    /// Both configured and advertised, read from the post-auth capability list, never an `*
    /// ENABLED` echo.
    can_idle: bool,
    targets: HashMap<MailboxId, Target>,
    pushed: Option<MailboxId>,
    suspended: bool,
}

impl Watcher {
    pub fn new(policy: WatchPolicy, capabilities: &Capabilities) -> Self {
        Self {
            policy,
            can_idle: policy.idle && capabilities.contains(Capability::Idle),
            targets: HashMap::new(),
            pushed: None,
            suspended: false,
        }
    }

    /// A server may come back with less (a failover to a node without `IDLE`).
    pub fn set_capabilities(&mut self, capabilities: &Capabilities) {
        self.can_idle = self.policy.idle && capabilities.contains(Capability::Idle);
    }

    pub fn policy(&self) -> WatchPolicy {
        self.policy
    }

    pub fn idles(&self) -> bool {
        self.can_idle
    }

    /// A second `Attention::Push` mailbox demotes the first to polling: a mailbox nobody watches is
    /// worse than one watched slowly.
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

    /// Returns a step at most once per mailbox until it is reported (`woke`, `observed` or
    /// `failed`): the duplicate-connection guarantee.
    pub fn next_push(&mut self, now: DateTime<Utc>) -> Watch {
        let Some(mailbox) = self.pushed else {
            return Watch::Wait { until: None };
        };
        self.step(mailbox, now)
    }

    /// The mailbox due soonest, never the pushed one, which belongs to `next_push`.
    pub fn next_poll(&mut self, now: DateTime<Utc>) -> Watch {
        if self.suspended || self.policy.manual {
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

    /// Any event is `Wake::Changed`: events say only that the mailbox moved. An empty slice is a
    /// quiet expiry or a cancelled `IDLE`.
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
        target.due_at = now;

        if events.is_empty() {
            return Wake::Quiet;
        }
        target.verified_at = Some(now);
        target.signature = None;
        Wake::Changed
    }

    /// The first look at a mailbox is always `Wake::Changed`: nothing local says what happened
    /// while we were gone.
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
        target.due_at = if idled { now } else { now + delta(interval) };

        if changed { Wake::Changed } else { Wake::Quiet }
    }

    /// The mailbox is released, not left in flight (a wedged target is silently unwatched), and
    /// held off one poll interval.
    pub fn failed(&mut self, mailbox: MailboxId, now: DateTime<Utc>) {
        let interval = self.policy.poll_interval;
        if let Some(target) = self.targets.get_mut(&mailbox) {
            target.in_flight = None;
            target.due_at = now + delta(interval);
        }
    }

    /// Reclaims every step whose caller vanished without reporting (#755): the engine's race drops
    /// the loser, so the target would stay outstanding and the inbox unwatched.
    /// Cancelling unwinds a half-run command; making the target due re-establishes the watch. Call
    /// only when the caller is known to be gone.
    pub fn interrupted(&mut self, now: DateTime<Utc>) {
        for target in self.targets.values_mut() {
            if let Some(cancel) = target.in_flight.take() {
                cancel.cancel();
                target.due_at = now;
                // The vanished command never reported what it would have caught; forgetting the
                // verification makes the next step a `STATUS`.
                target.verified_at = None;
            }
        }
    }

    /// Forgets every mailbox's verification without cancelling anything (#807). A queued write
    /// `SELECT`s the mailbox, so mail landing between the last `IDLE` and the write is in neither;
    /// the next decision becomes a `STATUS`. Safe while a step is outstanding.
    pub fn unverified(&mut self) {
        for target in self.targets.values_mut() {
            target.verified_at = None;
        }
    }

    /// Parks the watcher and cancels every outstanding command, so a suspending laptop does not
    /// leak an `IDLE`.
    /// Cancelled steps stay outstanding until their callers report, so resume cannot open a second
    /// connection.
    pub fn suspend(&mut self) {
        self.suspended = true;
        for target in self.targets.values() {
            if let Some(cancel) = &target.in_flight {
                cancel.cancel();
            }
        }
    }

    /// Un-parks the watcher and makes everything due at once: the time away is unknown.
    pub fn resume(&mut self, now: DateTime<Utc>) {
        self.suspended = false;
        for target in self.targets.values_mut() {
            target.due_at = now;
            target.verified_at = None;
        }
    }

    fn step(&mut self, mailbox: MailboxId, now: DateTime<Utc>) -> Watch {
        if self.suspended || self.policy.manual {
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

        // The watched mailbox idles unless it is overdue the `STATUS` that is the floor under a
        // deaf connection.
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

        // Carries a token because it marks the mailbox outstanding and is what a suspend fires at.
        target.in_flight = Some(CancelToken::new());
        Watch::Poll {
            mailbox,
            path: target.path.clone(),
        }
    }
}

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
    fn a_step_whose_caller_vanished_is_reclaimed_by_interrupted() {
        let mut watcher = Watcher::new(WatchPolicy::default(), &idling());
        let inbox = MailboxId::new(1);
        watcher.watch(inbox, "INBOX", Attention::Push);
        let now = DateTime::<Utc>::MIN_UTC;

        assert!(
            !matches!(watcher.next_push(now), Watch::Wait { .. }),
            "the pushed mailbox should be handed a step to run"
        );
        let outstanding = watcher.targets[&inbox]
            .in_flight
            .clone()
            .expect("a step is outstanding");
        assert!(
            matches!(watcher.next_push(now), Watch::Wait { until: None }),
            "an outstanding step blocks the lane; that is the wedge's first half"
        );

        watcher.interrupted(now);

        assert!(
            outstanding.is_cancelled(),
            "whatever half-run command the drop left behind has to be told to stop"
        );
        assert!(
            matches!(watcher.next_push(now), Watch::Poll { .. }),
            "a reclaimed mailbox is *reconciled*, not merely watched again: a \
             delivery during the gap is exactly what the dropped IDLE would \
             have caught, and only a STATUS can find it now"
        );
    }

    #[test]
    fn a_manual_policy_never_idles_or_polls_automatically() {
        let policy = WatchPolicy {
            manual: true,
            ..WatchPolicy::default()
        };
        let mut watcher = Watcher::new(policy, &idling());
        let inbox = MailboxId::new(1);
        watcher.watch(inbox, "INBOX", Attention::Push);
        let now = DateTime::<Utc>::MIN_UTC;

        assert!(
            matches!(watcher.next_push(now), Watch::Wait { until: None }),
            "manual mode must never idle automatically"
        );
        assert!(
            matches!(watcher.next_poll(now), Watch::Wait { until: None }),
            "manual mode must never poll automatically either"
        );
    }

    #[test]
    fn a_write_driving_the_connection_costs_the_watcher_its_evidence() {
        let mut watcher = Watcher::new(WatchPolicy::default(), &idling());
        let mailbox = MailboxId::new(1);
        let now = DateTime::<Utc>::MIN_UTC;
        watcher.watch(mailbox, "INBOX", Attention::Push);

        let status = MailboxStatus {
            path: "INBOX".to_owned(),
            generation: postio_model::Generation::new(1),
            uid_next: Uid::new(3),
            exists: 2,
            unseen: None,
            highest_mod_seq: None,
            permanent_flags: postio_model::FlagSet::default(),
            can_create_keywords: false,
            read_only: false,
        };
        watcher.observed(mailbox, &status, now);
        assert!(
            matches!(watcher.next_push(now), Watch::Idle { .. }),
            "a freshly verified mailbox idles; this test is about what \
             happens to that"
        );
        watcher.woke(mailbox, &[], now);

        watcher.unverified();

        assert!(
            matches!(watcher.next_push(now), Watch::Poll { .. }),
            "after something else drove the connection the mailbox is \
             *reconciled*, not idled at again -- only a STATUS can find what \
             the SELECT already swallowed"
        );
    }

    fn status() -> MailboxStatus {
        MailboxStatus {
            path: "INBOX".to_owned(),
            generation: postio_model::Generation::new(1),
            uid_next: Uid::new(3),
            exists: 2,
            unseen: None,
            highest_mod_seq: None,
            permanent_flags: postio_model::FlagSet::default(),
            can_create_keywords: false,
            read_only: false,
        }
    }

    #[test]
    fn the_default_policy_is_what_governs_the_wire() {
        let policy = WatchPolicy::default();
        assert_eq!(policy.idle_refresh, Duration::from_secs(9 * 60));
        assert_eq!(policy.poll_interval, Duration::from_secs(300));

        let mut watcher = Watcher::new(policy, &idling());
        let inbox = MailboxId::new(1);
        let now = DateTime::<Utc>::MIN_UTC;
        watcher.watch(inbox, "INBOX", Attention::Push);
        watcher.observed(inbox, &status(), now);
        let Watch::Idle { timeout, .. } = watcher.next_push(now) else {
            panic!("a verified mailbox on an IDLE server is idled at");
        };
        assert_eq!(timeout, Duration::from_secs(9 * 60));

        let mut watcher = Watcher::new(policy, &capabilities(&["IMAP4rev1"]));
        watcher.watch(inbox, "INBOX", Attention::Push);
        assert!(matches!(watcher.next_push(now), Watch::Poll { .. }));
        watcher.observed(inbox, &status(), now);
        let almost = now + TimeDelta::seconds(299);
        assert!(
            matches!(watcher.next_push(almost), Watch::Wait { .. }),
            "not yet: the interval is five minutes"
        );
        assert!(matches!(
            watcher.next_push(now + TimeDelta::seconds(300)),
            Watch::Poll { .. }
        ));
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
