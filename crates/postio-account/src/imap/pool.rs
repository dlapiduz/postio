//! Bounded, since servers cap connections per account; `IDLE` gets its own lane so watching never
//! costs a fetch slot.
//! Waiters are served interactive first, and background gives up after
//! `PoolConfig::acquire_timeout`. Health is checked on checkout, not on a timer: connections parked
//! past `idle_timeout` are replaced, and a session that failed transiently is discarded.

use std::collections::VecDeque;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::auth::{StoredPasswordSource, TokenSource};
use crate::backend::{BackendError, BackendResult, Capabilities};
use crate::secret::{AccountKey, Password, SecretStore};

use super::selection::Generations;
use super::{ConnectionSettings, Dispatch, ImapConnector, ImapSession};

/// Four covers an interactive fetch, a backfill, a queued mutation and the watcher, under every
/// provider's per-account limit.
pub const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// Comfortably under the 30 minutes RFC 3501 §5.4 allows a server to wait before dropping an idle
/// client.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// A bound on silence, never on duration: a large attachment over a slow link legitimately needs
/// minutes.
pub const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// The window in which an unnoticed `UIDVALIDITY` change could be acted on; not zero, since
/// backfill chunks follow each other in milliseconds.
pub const DEFAULT_SELECTION_MAX_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    #[default]
    Interactive,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolConfig {
    pub max_connections: usize,
    pub idle_timeout: Duration,
    pub acquire_timeout: Duration,
    /// With this off, watching competes for a general slot: on a two-connection server, watching or
    /// fetching.
    pub dedicate_watch_connection: bool,
    /// `Duration::ZERO` re-`SELECT`s before every operation: safest, most expensive.
    pub selection_max_age: Duration,
    /// Measured between reads, so a slow transfer is never mistaken for a hung one.
    /// `Duration::ZERO` waits forever.
    pub command_timeout: Duration,
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
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    pub in_use: usize,
    pub idle: usize,
    pub waiting: usize,
    pub opened: u64,
    pub capacity: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneKind {
    General,
    Watch,
}

struct Parked {
    session: ImapSession,
    since: Instant,
}

#[derive(Default)]
struct Lane {
    permits: usize,
    parked: Vec<Parked>,
    waiters: [VecDeque<(u64, oneshot::Sender<()>)>; Priority::COUNT],
}

impl Lane {
    fn with_permits(permits: usize) -> Self {
        Self {
            permits,
            ..Self::default()
        }
    }

    fn take_permit(&mut self, priority: Priority, ticket: u64) -> Option<oneshot::Receiver<()>> {
        if self.permits > 0 {
            self.permits -= 1;
            return None;
        }
        let (tx, rx) = oneshot::channel();
        self.waiters[priority.index()].push_back((ticket, tx));
        Some(rx)
    }

    /// The slot is transferred directly, so a woken waiter cannot lose it.
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

    fn take_parked(&mut self, idle_timeout: Duration, now: Instant) -> Option<ImapSession> {
        while let Some(parked) = self.parked.pop() {
            if now.duration_since(parked.since) < idle_timeout {
                return Some(parked.session);
            }
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
    in_use: usize,
    opened: u64,
    tickets: u64,
    closed: bool,
    capabilities: Option<Capabilities>,
}

/// Not `Clone`: two pools for one account would each think they owned the whole connection
/// allowance.
pub struct ConnectionPool {
    settings: ConnectionSettings,
    /// A `TokenSource`, not a `SecretStore`: `invalidate` is what a rejected bearer token needs and
    /// a rejected password ignores (#193).
    tokens: Arc<dyn TokenSource>,
    key: AccountKey,
    connector: Arc<dyn ImapConnector>,
    config: PoolConfig,
    watch_dedicated: bool,
    /// Shared so one connection discovering a renumber stops the others acting on cached
    /// generations.
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
    pub fn new(
        settings: ConnectionSettings,
        key: AccountKey,
        store: Arc<dyn SecretStore>,
        connector: Arc<dyn ImapConnector>,
        config: PoolConfig,
    ) -> Self {
        Self::with_token_source(
            settings,
            key,
            Arc::new(StoredPasswordSource::new(store)),
            connector,
            config,
        )
    }

    /// What an OAuth account needs: this pool sees that the server refused what the source handed
    /// over (ADR 0006 Q5).
    pub fn with_token_source(
        settings: ConnectionSettings,
        key: AccountKey,
        tokens: Arc<dyn TokenSource>,
        connector: Arc<dyn ImapConnector>,
        config: PoolConfig,
    ) -> Self {
        let capacity = config.max_connections.max(1);
        // On a one-connection budget there is nothing to dedicate, so watching shares the general
        // lane.
        let watch_slots = usize::from(config.dedicate_watch_connection && capacity > 1);

        Self {
            settings,
            tokens,
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

    pub fn settings(&self) -> &ConnectionSettings {
        &self.settings
    }

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

    /// `None` before the first connection: they are read from the server, never assumed.
    pub fn capabilities(&self) -> Option<Capabilities> {
        self.lock().capabilities.clone()
    }

    pub fn dispatch(&self) -> Option<Dispatch> {
        self.capabilities().map(Dispatch::new)
    }

    pub async fn execute<T>(
        &self,
        priority: Priority,
        operation: impl AsyncFnOnce(&mut ImapSession) -> BackendResult<T>,
    ) -> BackendResult<T> {
        let connection = self.acquire(priority).await?;
        Self::run_on(connection, operation).await
    }

    /// The connection is condemned before the operation and reprieved only by a result proving the
    /// exchange finished: if the future is dropped, the session may hold half a response or be
    /// inside `IDLE`.
    async fn run_on<T>(
        mut connection: PooledSession<'_>,
        operation: impl AsyncFnOnce(&mut ImapSession) -> BackendResult<T>,
    ) -> BackendResult<T> {
        connection.discard();
        let result = operation(&mut connection).await;
        match result.as_ref() {
            Err(error) if error.is_transient() => Self::discarding(error),
            _ => connection.discard = false,
        }
        result
    }

    /// A parse can be a reason to discard (a lost `UIDVALIDITY` line, a short `UID SEARCH`), and
    /// this line is the only trace; `warn` because a server or protocol library misbehaved. No
    /// mailbox path or content.
    fn discarding(error: &BackendError) {
        tracing::warn!(%error, "discarding the connection that produced this");
    }

    /// Separate from `execute` so a long `IDLE` never occupies a slot interactive work needs.
    pub async fn watch<T>(
        &self,
        operation: impl AsyncFnOnce(&mut ImapSession) -> BackendResult<T>,
    ) -> BackendResult<T> {
        let connection = self
            .acquire_lane(LaneKind::Watch, Priority::Interactive)
            .await?;
        Self::run_on(connection, operation).await
    }

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

    /// Logs the endpoint (`host:port`), never the password.
    #[tracing::instrument(skip_all, fields(endpoint = %self.settings.endpoint()))]
    async fn open(&self) -> BackendResult<ImapSession> {
        // In `auth` because SMTP needs the same discipline; against a revoked grant one retry, not
        // a loop (ADR 0006 Q5).
        crate::auth::with_credential(
            self.tokens.as_ref(),
            &self.key,
            BackendError::is_authentication_failure,
            |credential| async move { self.open_with(&credential).await },
        )
        .await
        .map_err(|error| {
            // Redacted: the error names the account for the user's screen, but a log gets pasted
            // into issues, so the local part goes.
            tracing::error!(
                error = %postio_model::address::redact_addresses(&error.to_string()),
                "no credential for this account"
            );
            BackendError::from(error)
        })?
    }

    async fn open_with(&self, password: &Password) -> BackendResult<ImapSession> {
        tracing::debug!("connecting");
        let mut session =
            match ImapSession::open(&self.settings, password, self.connector.as_ref()).await {
                Ok(session) => session,
                Err(error) => {
                    tracing::warn!(
                        error = %postio_model::address::redact_addresses(&error.to_string()),
                        "connection or authentication failed"
                    );
                    return Err(error);
                }
            };
        tracing::info!(
            capabilities = ?session.capabilities().names(),
            "connected and authenticated"
        );
        session.set_selection_policy(Arc::clone(&self.generations), self.config.selection_max_age);
        session.set_command_timeout(self.config.command_timeout);

        let mut inner = self.lock();
        inner.opened += 1;
        // Later connections refresh it: a provider may change its mind, and a stale capability set
        // is what ADR 0001 is about.
        inner.capabilities = Some(session.capabilities().clone());
        Ok(session)
    }

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

    /// "Hold nothing open" (after a disconnect, suspend or network loss), not shut down; that is
    /// `close`.
    pub fn drop_idle(&self) {
        let mut inner = self.lock();
        let inner = &mut *inner;
        for lane in [&mut inner.general, &mut inner.watch] {
            lane.parked.clear();
        }
    }

    /// Checked-out connections are dropped rather than parked when their holder is done.
    pub fn close(&self) {
        let mut inner = self.lock();
        inner.closed = true;
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

pub struct PooledSession<'a> {
    pool: &'a ConnectionPool,
    lane: LaneKind,
    session: Option<ImapSession>,
    discard: bool,
}

impl PooledSession<'_> {
    /// Call by hand after driving a connection into an unexpected state: an aborted literal, a
    /// half-read response.
    pub fn discard(&mut self) {
        self.discard = true;
    }

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
