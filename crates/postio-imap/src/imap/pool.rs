//! A bounded pool of authenticated sessions.
//!
//! Two facts shape this. Servers cap how many connections one account may hold
//! open — iCloud is not generous — so the pool is bounded and the bound is
//! respected under concurrent load. And `IDLE` occupies a connection for
//! minutes at a time doing nothing, so it gets a lane of its own; otherwise
//! watching the inbox would cost a slot that fetching needs.
//!
//! # Fairness
//!
//! Waiters are served **interactive first**. Opening a thread must not queue
//! behind a ten-thousand-message backfill that grabbed every slot, and since a
//! backfill is made of many small acquisitions rather than one long one, it
//! yields naturally between chunks. Background work is served whenever no
//! interactive work is waiting, and gives up after
//! [`PoolConfig::acquire_timeout`] rather than waiting forever, so sustained
//! interactive load surfaces as a visible error instead of a silent stall.
//!
//! # Health
//!
//! A connection is checked when it is taken out, not on a timer: anything
//! parked longer than [`PoolConfig::idle_timeout`] is closed and replaced
//! rather than probed, because a probe costs a round trip on every single
//! acquisition and the answer is stale the moment it arrives. A session whose
//! command failed transiently is discarded rather than parked — that is what
//! makes a dropped connection cost one failed operation instead of every
//! subsequent one.

use std::collections::VecDeque;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::backend::{BackendError, BackendResult, Capabilities};
use crate::secret::{AccountKey, SecretStore};

use super::selection::Generations;
use super::{ConnectionSettings, Dispatch, ImapConnector, ImapSession};

/// How many connections a pool opens by default, the watcher included.
///
/// Deliberately small. Four is enough for an interactive fetch, a background
/// backfill, a queued mutation and the watcher, and it leaves room under every
/// provider's per-account limit.
pub const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// How long a parked connection may sit before it is closed rather than
/// reused. Comfortably under the 30 minutes RFC 3501 §5.4 allows a server to
/// wait before dropping an idle client.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// How long an acquisition waits for a slot before giving up.
pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a command may go without a byte from the server before it is
/// given up on.
///
/// The bound is on silence, never on how long an exchange takes: a large
/// attachment over a slow link legitimately needs minutes, and a deadline
/// tight enough to catch a hung server would kill it. Sixty seconds is far
/// longer than any healthy server pauses mid-response and far shorter than
/// forever, which is what this replaced.
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// How often a watcher winds `IDLE` down and arms it again.
///
/// A server may drop an `IDLE` that has run too long — RFC 2177 §3 allows it
/// after 29 minutes, and NAT middle-boxes are far less patient — and a
/// watcher that does not re-arm inside that window goes deaf with no error
/// anywhere. Ten minutes is six round trips an hour per mailbox, against the
/// hundred and twenty `io-imap`'s own 29-second default would cost a laptop
/// on battery.
pub const DEFAULT_WATCH_REFRESH: Duration = Duration::from_secs(10 * 60);

/// How often a server without `IDLE` is asked for a `STATUS` instead.
///
/// This is the latency of "new mail appears" on such a server, so it is a
/// user-visible number rather than a housekeeping one.
pub const DEFAULT_WATCH_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// How long a cached mailbox selection may answer before the server is asked
/// to confirm it again.
///
/// This is the window in which a `UIDVALIDITY` change nobody has noticed yet
/// could still be acted on, so it is short; and it is not zero because the
/// chunks of a backfill follow each other in milliseconds and re-selecting
/// before each of them would double that operation's round trips. Thirty
/// seconds costs an interactive user at most one extra `SELECT` per mailbox
/// per half-minute and a backfill nothing at all. See
/// [`selection`](super::selection).
pub const DEFAULT_SELECTION_MAX_AGE: Duration = Duration::from_secs(30);

/// What a piece of work is competing for.
///
/// The distinction is whether somebody is looking at the result: an
/// [`Interactive`](Priority::Interactive) acquisition is a keystroke away from
/// a user, a [`Background`](Priority::Background) one is a backfill nobody is
/// waiting on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// The user is waiting. Served first.
    #[default]
    Interactive,
    /// Nobody is waiting. Served when nothing interactive is queued.
    Background,
}

impl Priority {
    const COUNT: usize = 2;

    fn index(self) -> usize {
        match self {
            Self::Interactive => 0,
            Self::Background => 1,
        }
    }
}

/// How the pool is sized and aged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolConfig {
    /// The most connections to hold open at once, including the watcher.
    pub max_connections: usize,
    /// How long a parked connection may sit before it is closed.
    pub idle_timeout: Duration,
    /// How long an acquisition waits for a slot.
    pub acquire_timeout: Duration,
    /// Whether one connection is reserved for `IDLE`.
    ///
    /// With this off, watching competes for a general slot, which on a
    /// two-connection server is the difference between watching *or* fetching.
    pub dedicate_watch_connection: bool,
    /// How long a cached mailbox selection may answer before the server has
    /// to confirm it again.
    ///
    /// [`Duration::ZERO`] re-`SELECT`s before every operation, which is the
    /// safest and the most expensive. See [`DEFAULT_SELECTION_MAX_AGE`].
    pub selection_max_age: Duration,
    /// How long a command may go without a byte from the server.
    ///
    /// Measured between reads, not across the command, so a slow transfer is
    /// never mistaken for a hung one. [`Duration::ZERO`] waits forever, which
    /// is what the pool before this bound did. See
    /// [`DEFAULT_COMMAND_TIMEOUT`].
    pub command_timeout: Duration,
    /// How often a watcher re-arms `IDLE`. See [`DEFAULT_WATCH_REFRESH`].
    pub watch_refresh: Duration,
    /// How often a server without `IDLE` is polled with `STATUS`. See
    /// [`DEFAULT_WATCH_POLL_INTERVAL`].
    pub watch_poll_interval: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_MAX_CONNECTIONS,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            dedicate_watch_connection: true,
            selection_max_age: DEFAULT_SELECTION_MAX_AGE,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            watch_refresh: DEFAULT_WATCH_REFRESH,
            watch_poll_interval: DEFAULT_WATCH_POLL_INTERVAL,
        }
    }
}

/// A snapshot of what the pool is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// Connections currently checked out.
    pub in_use: usize,
    /// Connections open but parked.
    pub idle: usize,
    /// Acquisitions waiting for a slot.
    pub waiting: usize,
    /// Connections opened over the pool's lifetime. Grows when a dead
    /// connection is replaced, which is what makes replacement observable.
    pub opened: u64,
    /// The configured ceiling.
    pub capacity: usize,
}

/// Which lane a connection belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneKind {
    /// Commands: fetch, store, move, list.
    General,
    /// The long-lived `IDLE` connection.
    Watch,
}

/// A connection parked between uses.
struct Parked {
    session: ImapSession,
    since: Instant,
}

/// One lane's slots, parked connections and waiters.
#[derive(Default)]
struct Lane {
    /// Slots not currently held by anybody.
    permits: usize,
    /// Open connections nobody is using.
    parked: Vec<Parked>,
    /// Waiters, highest priority first.
    waiters: [VecDeque<(u64, oneshot::Sender<()>)>; Priority::COUNT],
}

impl Lane {
    fn with_permits(permits: usize) -> Self {
        Self {
            permits,
            ..Self::default()
        }
    }

    /// Takes a slot, or queues a waiter for one.
    fn take_permit(&mut self, priority: Priority, ticket: u64) -> Option<oneshot::Receiver<()>> {
        if self.permits > 0 {
            self.permits -= 1;
            return None;
        }
        let (tx, rx) = oneshot::channel();
        self.waiters[priority.index()].push_back((ticket, tx));
        Some(rx)
    }

    /// Hands a slot to the next waiter, or puts it back.
    ///
    /// The slot is transferred directly rather than released and re-contended
    /// for, so a woken waiter cannot lose the slot it was woken for.
    fn release_permit(&mut self) {
        for queue in &mut self.waiters {
            while let Some((_, waker)) = queue.pop_front() {
                // A closed receiver is a waiter that timed out; skip it and
                // keep the slot moving.
                if waker.send(()).is_ok() {
                    return;
                }
            }
        }
        self.permits += 1;
    }

    /// Removes an abandoned waiter. Returns whether it was still queued —
    /// `false` means a slot was already handed to it and must be released.
    fn cancel(&mut self, ticket: u64) -> bool {
        for queue in &mut self.waiters {
            if let Some(at) = queue.iter().position(|(id, _)| *id == ticket) {
                queue.remove(at);
                return true;
            }
        }
        false
    }

    /// Takes a parked connection, dropping any that sat too long.
    fn take_parked(&mut self, idle_timeout: Duration, now: Instant) -> Option<ImapSession> {
        while let Some(parked) = self.parked.pop() {
            if now.duration_since(parked.since) < idle_timeout {
                return Some(parked.session);
            }
            // Too old to trust. Dropping the session closes the socket; a
            // polite LOGOUT would need an await we cannot make here, and the
            // server has almost certainly dropped it already.
        }
        None
    }

    fn waiting(&self) -> usize {
        self.waiters.iter().map(VecDeque::len).sum()
    }
}

struct PoolInner {
    general: Lane,
    watch: Lane,
    /// Connections handed out and not yet returned.
    in_use: usize,
    opened: u64,
    tickets: u64,
    closed: bool,
    capabilities: Option<Capabilities>,
}

/// A bounded pool of authenticated IMAP sessions for one account.
///
/// Cloning is not offered: wrap it in an `Arc`. One pool is one account's
/// share of one server's connection budget, and two pools for the same account
/// would each think they owned the whole allowance.
pub struct ConnectionPool {
    settings: ConnectionSettings,
    store: Arc<dyn SecretStore>,
    key: AccountKey,
    connector: Arc<dyn ImapConnector>,
    config: PoolConfig,
    /// Whether the watch lane has a slot of its own; see [`ConnectionPool::new`].
    watch_dedicated: bool,
    /// What every session in this pool has observed about each mailbox's UID
    /// generation. Shared so that one connection discovering a renumber stops
    /// the others from acting on what they cached before it.
    generations: Arc<Generations>,
    inner: Mutex<PoolInner>,
}

impl fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionPool")
            .field("endpoint", &self.settings.endpoint())
            .field("account", &self.key.account())
            .field("stats", &self.stats())
            .finish()
    }
}

impl ConnectionPool {
    /// Builds a pool. No connection is opened until one is asked for.
    pub fn new(
        settings: ConnectionSettings,
        key: AccountKey,
        store: Arc<dyn SecretStore>,
        connector: Arc<dyn ImapConnector>,
        config: PoolConfig,
    ) -> Self {
        let capacity = config.max_connections.max(1);
        // A dedicated watcher needs a connection of its own *and* one left
        // over for commands. On a one-connection budget there is nothing to
        // dedicate, so watching shares the general lane rather than waiting
        // forever for a slot that was never created.
        let watch_slots = usize::from(config.dedicate_watch_connection && capacity > 1);

        Self {
            settings,
            store,
            key,
            connector,
            watch_dedicated: watch_slots > 0,
            generations: Arc::new(Generations::new()),
            config: PoolConfig {
                max_connections: capacity,
                ..config
            },
            inner: Mutex::new(PoolInner {
                general: Lane::with_permits(capacity - watch_slots),
                watch: Lane::with_permits(watch_slots),
                in_use: 0,
                opened: 0,
                tickets: 0,
                closed: false,
                capabilities: None,
            }),
        }
    }

    /// What the pool is doing right now.
    pub fn stats(&self) -> PoolStats {
        let inner = self.lock();
        PoolStats {
            in_use: inner.in_use,
            idle: inner.general.parked.len() + inner.watch.parked.len(),
            waiting: inner.general.waiting() + inner.watch.waiting(),
            opened: inner.opened,
            capacity: self.config.max_connections,
        }
    }

    /// The capabilities the server advertised, once a connection has been
    /// opened. `None` before that: they are read from the server, never
    /// assumed.
    pub fn capabilities(&self) -> Option<Capabilities> {
        self.lock().capabilities.clone()
    }

    /// The command choices this server's capabilities imply, once known.
    pub fn dispatch(&self) -> Option<Dispatch> {
        self.capabilities().map(Dispatch::new)
    }

    /// How often a watcher on this pool re-arms `IDLE`.
    pub(super) fn watch_refresh(&self) -> Duration {
        self.config.watch_refresh
    }

    /// How often a watcher polls a server that has no `IDLE`.
    pub(super) fn watch_poll_interval(&self) -> Duration {
        self.config.watch_poll_interval
    }

    /// Runs one operation on a pooled connection.
    ///
    /// A connection whose operation failed transiently is discarded rather
    /// than parked: a dropped socket should cost one failed operation, not
    /// every operation after it.
    pub async fn execute<T>(
        &self,
        priority: Priority,
        operation: impl AsyncFnOnce(&mut ImapSession) -> BackendResult<T>,
    ) -> BackendResult<T> {
        let mut connection = self.acquire(priority).await?;
        let result = operation(&mut connection).await;
        if result
            .as_ref()
            .err()
            .is_some_and(BackendError::is_transient)
        {
            connection.discard();
        }
        result
    }

    /// Runs one operation on the connection reserved for watching.
    ///
    /// Separate from [`execute`](Self::execute) so a long `IDLE` never
    /// occupies a slot that interactive work needs.
    pub async fn watch<T>(
        &self,
        operation: impl AsyncFnOnce(&mut ImapSession) -> BackendResult<T>,
    ) -> BackendResult<T> {
        let mut connection = self
            .acquire_lane(LaneKind::Watch, Priority::Interactive)
            .await?;
        let result = operation(&mut connection).await;
        if result
            .as_ref()
            .err()
            .is_some_and(BackendError::is_transient)
        {
            connection.discard();
        }
        result
    }

    /// Checks out a connection, opening one if the pool has room.
    pub async fn acquire(&self, priority: Priority) -> BackendResult<PooledSession<'_>> {
        self.acquire_lane(LaneKind::General, priority).await
    }

    async fn acquire_lane(
        &self,
        lane: LaneKind,
        priority: Priority,
    ) -> BackendResult<PooledSession<'_>> {
        let lane = match lane {
            LaneKind::Watch if !self.watch_dedicated => LaneKind::General,
            lane => lane,
        };

        let waiting = {
            let mut inner = self.lock();
            inner.guard_open()?;
            let ticket = inner.next_ticket();
            (inner.lane_mut(lane).take_permit(priority, ticket)).map(|rx| (ticket, rx))
        };

        if let Some((ticket, receiver)) = waiting {
            match tokio::time::timeout(self.config.acquire_timeout, receiver).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    // Every sender is gone: the pool closed underneath us.
                    return Err(self.closed_error());
                }
                Err(_) => {
                    let mut inner = self.lock();
                    if !inner.lane_mut(lane).cancel(ticket) {
                        // The slot arrived just as we gave up on it; pass it on
                        // rather than leaking it.
                        inner.lane_mut(lane).release_permit();
                    }
                    return Err(BackendError::TimedOut {
                        context: format!(
                            "waiting for a connection to {}",
                            self.settings.endpoint()
                        ),
                        after: self.config.acquire_timeout,
                    });
                }
            }
        }

        // The slot is ours from here; every path out must give it back.
        let parked = {
            let mut inner = self.lock();
            if inner.closed {
                inner.lane_mut(lane).release_permit();
                return Err(self.closed_error());
            }
            inner
                .lane_mut(lane)
                .take_parked(self.config.idle_timeout, Instant::now())
        };

        let session = match parked {
            Some(session) => session,
            None => match self.open().await {
                Ok(session) => session,
                Err(error) => {
                    self.lock().lane_mut(lane).release_permit();
                    return Err(error);
                }
            },
        };

        self.lock().in_use += 1;
        Ok(PooledSession {
            pool: self,
            lane,
            session: Some(session),
            discard: false,
        })
    }

    /// Opens and authenticates one connection.
    async fn open(&self) -> BackendResult<ImapSession> {
        let password = self.store.retrieve(&self.key).await?;
        let mut session =
            ImapSession::open(&self.settings, &password, self.connector.as_ref()).await?;
        session.set_selection_policy(Arc::clone(&self.generations), self.config.selection_max_age);
        session.set_command_timeout(self.config.command_timeout);

        let mut inner = self.lock();
        inner.opened += 1;
        // The first connection settles what this server can do. Later ones
        // refresh it: a provider is allowed to change its mind, and a
        // capability set that silently went stale is the failure mode ADR 0001
        // is about.
        inner.capabilities = Some(session.capabilities().clone());
        Ok(session)
    }

    /// Returns a connection to its lane, or drops it.
    fn give_back(&self, lane: LaneKind, session: Option<ImapSession>, discard: bool) {
        let mut inner = self.lock();
        inner.in_use = inner.in_use.saturating_sub(1);

        if let Some(session) = session
            && !discard
            && !inner.closed
        {
            inner.lane_mut(lane).parked.push(Parked {
                session,
                since: Instant::now(),
            });
        }

        inner.lane_mut(lane).release_permit();
    }

    /// Closes the pool: parked connections are dropped and new acquisitions
    /// are refused.
    ///
    /// Connections currently checked out are not interrupted; they are dropped
    /// rather than parked when their holder is done.
    pub fn close(&self) {
        let mut inner = self.lock();
        inner.closed = true;
        // One reborrow through the guard, so both lanes can be taken at once.
        let inner = &mut *inner;
        for lane in [&mut inner.general, &mut inner.watch] {
            lane.parked.clear();
            for queue in &mut lane.waiters {
                queue.clear();
            }
        }
    }

    fn closed_error(&self) -> BackendError {
        BackendError::NotConnected {
            context: format!(
                "the connection pool for {} is closed",
                self.settings.endpoint()
            ),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PoolInner> {
        self.inner.lock().expect("connection pool mutex")
    }
}

impl PoolInner {
    fn lane_mut(&mut self, lane: LaneKind) -> &mut Lane {
        match lane {
            LaneKind::General => &mut self.general,
            LaneKind::Watch => &mut self.watch,
        }
    }

    fn next_ticket(&mut self) -> u64 {
        self.tickets += 1;
        self.tickets
    }

    fn guard_open(&self) -> BackendResult<()> {
        if self.closed {
            return Err(BackendError::NotConnected {
                context: "the connection pool is closed".to_owned(),
            });
        }
        Ok(())
    }
}

/// A connection checked out of a [`ConnectionPool`].
///
/// Returns to the pool when dropped, unless [`discard`](Self::discard) was
/// called or the pool has been closed.
pub struct PooledSession<'a> {
    pool: &'a ConnectionPool,
    lane: LaneKind,
    session: Option<ImapSession>,
    discard: bool,
}

impl PooledSession<'_> {
    /// Marks this connection as not worth reusing.
    ///
    /// [`ConnectionPool::execute`] calls this for you on a transient failure.
    /// Call it by hand after driving a connection into a state the next caller
    /// would not expect — an aborted literal, a half-read response.
    pub fn discard(&mut self) {
        self.discard = true;
    }

    /// Whether this connection will be dropped rather than parked.
    pub fn is_discarded(&self) -> bool {
        self.discard
    }
}

impl fmt::Debug for PooledSession<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PooledSession")
            .field("lane", &self.lane)
            .field("discard", &self.discard)
            .finish_non_exhaustive()
    }
}

impl Deref for PooledSession<'_> {
    type Target = ImapSession;

    fn deref(&self) -> &Self::Target {
        self.session.as_ref().expect("a checked-out session")
    }
}

impl DerefMut for PooledSession<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.session.as_mut().expect("a checked-out session")
    }
}

impl Drop for PooledSession<'_> {
    fn drop(&mut self) {
        self.pool
            .give_back(self.lane, self.session.take(), self.discard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lane_hands_a_released_slot_straight_to_the_highest_priority_waiter() {
        let mut lane = Lane::with_permits(1);

        assert!(lane.take_permit(Priority::Interactive, 1).is_none());
        let mut background = lane.take_permit(Priority::Background, 2).expect("queued");
        let mut interactive = lane.take_permit(Priority::Interactive, 3).expect("queued");

        lane.release_permit();

        // The interactive waiter arrived second and is served first.
        assert!(interactive.is_terminated_or_ready());
        assert!(!background.is_terminated_or_ready());
    }

    #[test]
    fn a_cancelled_waiter_is_removed_from_its_queue() {
        let mut lane = Lane::with_permits(0);
        let _rx = lane.take_permit(Priority::Background, 7);

        assert!(lane.cancel(7));
        assert!(!lane.cancel(7));
    }

    #[test]
    fn releasing_with_nobody_waiting_puts_the_slot_back() {
        let mut lane = Lane::with_permits(1);
        assert!(lane.take_permit(Priority::Interactive, 1).is_none());
        assert_eq!(lane.permits, 0);

        lane.release_permit();

        assert_eq!(lane.permits, 1);
    }

    /// `oneshot::Receiver` has no "is it ready" method, so borrow one.
    trait ReceiverReady {
        fn is_terminated_or_ready(&mut self) -> bool;
    }

    impl ReceiverReady for oneshot::Receiver<()> {
        fn is_terminated_or_ready(&mut self) -> bool {
            self.try_recv().is_ok()
        }
    }
}
