//! The sync engine, driven from one thread that owns a connection.
//!
//! `postio-sync` had `Drainer` and `Backfill` and nothing called either: the
//! operation queue filled up locally and never reached a server, and message
//! bodies were never fetched. This is the loop that runs them.
//!
//! # Why it is a thread and not a task
//!
//! `Drainer::drain` is async and borrows a `rusqlite::Connection` across its
//! awaits. `Connection` is `!Sync`, so `&Connection` is `!Send`, so the future
//! is `!Send` and `tokio::spawn` will not take it — and neither will the
//! command bus, whose handlers are boxed `Send` futures.
//!
//! That is not a wart to route around. A drain is a long, stateful,
//! *sequential* thing: one connection, one queue, one order. So it gets a
//! thread of its own running a current-thread runtime, keeps its connection
//! there, and takes work over a channel. Nothing about it crosses a thread
//! boundary while borrowed, and every caller awaits a reply rather than
//! blocking — which is the rule that matters, because the caller is the UI.
//!
//! # What runs here
//!
//! * **Draining** the operation queue for an account: flags, moves, deletes,
//!   expunges over IMAP, and sends over SMTP.
//! * **Backfilling** message bodies: seeded per mailbox after a sync, and
//!   jumped to the front when the reading pane opens something.
//! * **Staying connected**: a `Supervisor` per engine, polled on a timer and
//!   told directly by whatever hit a broken connection, so a dropped session
//!   is noticed by the operation that found it rather than by the next tick.
//!
//! Both report back as [`Event`]s on the sink they were given, so the UI
//! learns what happened the same way it learns everything else.

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use postio_imap::backend::{BackendError, MailBackend};
use postio_imap::secret::SecretStore;
use postio_model::RoleOverrides;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_smtp::transport::SmtpConnector;
use postio_storage::repository::{
    MailboxRepository, OperationQueueRepository, SyncStateRepository,
};
use postio_storage::{BlobStore, Database, Pool, PooledConnection};
use postio_sync::initial::Progress;
use postio_sync::status::StatusTracker;
use postio_sync::{
    Attention, Backfill, BackfillPolicy, BackfillProgress, Drainer, ReconnectPolicy, RetryPolicy,
    SmtpContext, Supervisor, SyncError, SyncStatus, Wake, Watch, WatchPolicy, Watcher, backfill,
    discover, initial, resync,
};
// The crate root's `Outcome` is the *resync* one; a body has its own.
use postio_sync::backfill::Outcome;

/// What the connection is doing, and what the operating system says about the
/// network.
///
/// Re-exported rather than mirrored: whoever holds an [`Engine`] is the
/// composition root, which depends on `postio-sync` anyway, and a second
/// enum saying the same thing is a second enum to keep in step. The *frontend*
/// never sees these — this whole module is behind the `runtime` feature, which
/// `postio-gtk` cannot enable.
pub use postio_sync::{Blocker, Link, NetworkState};

use postio_core::Event;
use postio_core::bridge::EventSink;

/// What one drain pass did.
///
/// Core's own summary of `postio_sync::DrainReport`, so the sync engine's
/// types stay behind this boundary the way the storage types do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainSummary {
    /// Rows the server accepted.
    pub applied: usize,
    /// Rows that needed no round trip because they cancelled each other out.
    pub coalesced: usize,
    /// Rows whose message was not where the operation expected it.
    pub obsolete: usize,
    /// Rows waiting for a retry.
    pub deferred: usize,
    /// Rows given up on, each with the reason the user should see.
    pub failed: Vec<String>,
    /// Mailboxes whose local state is now known to disagree with the server.
    pub needs_resync: Vec<MailboxId>,
}

impl DrainSummary {
    /// Whether anything at all happened.
    pub fn is_empty(&self) -> bool {
        self.applied == 0
            && self.coalesced == 0
            && self.obsolete == 0
            && self.deferred == 0
            && self.failed.is_empty()
    }
}

/// What one sync pass did to a mailbox.
///
/// Core's own summary, so `postio-sync`'s report types stay behind this
/// boundary the way the storage types do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSummary {
    /// Messages that were not known locally before this pass.
    pub inserted: usize,
    /// Messages already present that this pass wrote again.
    pub updated: usize,
    /// Messages filed into a thread during this pass.
    pub threaded: usize,
    /// Whether the whole mailbox had to be re-enumerated.
    pub full: bool,
}

impl SyncSummary {
    /// Whether the local store moved at all.
    pub fn changed(&self) -> bool {
        self.inserted > 0 || self.updated > 0 || self.threaded > 0
    }
}

/// Work the engine could not do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    message: String,
}

impl EngineError {
    /// The failure, phrased for the user. Never contains a secret.
    pub fn message(&self) -> &str {
        &self.message
    }

    fn new(message: impl Into<String>) -> Self {
        EngineError {
            message: message.into(),
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}

/// Everything the engine needs to exist.
///
/// Assembled by whoever owns the account — the composition root — because
/// choosing a transport, a keyring and a blob directory is exactly the kind of
/// decision a runtime should be handed rather than make.
pub struct EngineParts {
    /// The account this engine is for.
    ///
    /// One engine per account, because a second account is a second server,
    /// a second password and a second link to keep up. Nothing here is shared
    /// between them but the database.
    pub account: AccountId,
    /// The local store.
    pub database: Database,
    /// Where attachment bytes are read from and the sent copy is written to.
    pub blobs: BlobStore,
    /// The IMAP side: flags, moves, deletes, expunges, bodies.
    pub backend: Arc<dyn MailBackend>,
    /// The SMTP side. Without it `Operation::Send` fails outright, which is
    /// not a bug — a queue with nothing to send through cannot send.
    pub smtp: Arc<dyn SmtpConnector>,
    /// Where the account's password lives. The same one IMAP uses.
    pub secrets: Arc<dyn SecretStore>,
    /// Where the engine reports what it did.
    pub events: EventSink,
    /// How hard to retry a failed operation.
    pub retry: RetryPolicy,
    /// How eagerly to fetch bodies nobody has asked for yet.
    pub backfill: BackfillPolicy,
    /// How long to wait before trying a dropped connection again.
    pub reconnect: ReconnectPolicy,
    /// How closely to watch for mail arriving.
    pub watch: WatchPolicy,
    /// Where the engine learns that the machine's network came or went.
    pub network: NetworkSource,
    /// Folders the user has assigned a role to by hand, from `[mailboxes]`.
    ///
    /// Carried rather than read here for the reason every other policy on
    /// this struct is: which folder is the archive is a fact about *this
    /// installation*, and an engine that reached for a config file itself
    /// could not be driven by a test. Empty is the ordinary case and costs
    /// nothing — see [`RoleOverrides`].
    pub mailbox_roles: RoleOverrides,
}

/// Who tells the engine about the network.
///
/// Explicit rather than "listen if the bus is there", because a test that
/// quietly opened a D-Bus connection would be a test that behaves differently
/// on a developer's desktop and on a CI runner. The application asks for
/// [`NetworkSource::NetworkManager`]; everything else leaves it alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkSource {
    /// Nobody. The link is judged entirely by whether connections work, which
    /// is correct but waits out a backoff the machine could have skipped.
    #[default]
    Ignored,
    /// NetworkManager over the system bus. Falls back to [`Self::Ignored`]
    /// behaviour on a machine that does not run it.
    NetworkManager,
}

/// One unit of work for the engine's thread.
enum Job {
    Drain {
        reply: tokio::sync::oneshot::Sender<Result<DrainSummary, EngineError>>,
    },
    /// The user supplied a new password, so a blocked link is worth trying.
    RetryNow {
        reply: tokio::sync::oneshot::Sender<Link>,
    },
    /// The operating system changed its mind about the network.
    SetNetwork {
        state: NetworkState,
        reply: tokio::sync::oneshot::Sender<Link>,
    },
    /// What the link is doing right now.
    LinkState {
        reply: tokio::sync::oneshot::Sender<Link>,
    },
    /// How far the backfill has got.
    BackfillProgress {
        reply: tokio::sync::oneshot::Sender<BackfillProgress>,
    },
    /// Bring one mailbox in line with the server.
    Sync {
        mailbox: MailboxId,
        reply: tokio::sync::oneshot::Sender<Result<SyncSummary, EngineError>>,
    },
    SeedBackfill {
        mailbox: MailboxId,
        limit: u32,
        reply: tokio::sync::oneshot::Sender<Result<usize, EngineError>>,
    },
    RequestBody {
        message: MessageId,
        reply: tokio::sync::oneshot::Sender<Result<bool, EngineError>>,
    },
}

/// A handle to the sync engine.
///
/// Cloning gives another handle to the same thread. Dropping every handle
/// stops it.
#[derive(Debug, Clone)]
pub struct Engine {
    jobs: async_channel::Sender<Job>,
}

impl fmt::Debug for Job {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Job::Drain { .. } => formatter.write_str("Drain"),
            Job::RetryNow { .. } => formatter.write_str("RetryNow"),
            Job::SetNetwork { state, .. } => write!(formatter, "SetNetwork({state:?})"),
            Job::LinkState { .. } => formatter.write_str("LinkState"),
            Job::BackfillProgress { .. } => formatter.write_str("BackfillProgress"),
            Job::Sync { mailbox, .. } => write!(formatter, "Sync({mailbox})"),
            Job::SeedBackfill { mailbox, .. } => write!(formatter, "SeedBackfill({mailbox})"),
            Job::RequestBody { message, .. } => write!(formatter, "RequestBody({message})"),
        }
    }
}

impl Engine {
    /// Start the engine on a thread of its own.
    ///
    /// Returns as soon as the thread is running; nothing has been drained yet.
    pub fn spawn(parts: EngineParts) -> Result<Engine, EngineError> {
        // Unbounded because the sender is the UI and it must never block on
        // the engine. What arrives is a handful of small jobs, not a stream.
        let (jobs, inbox) = async_channel::unbounded::<Job>();
        let pool = parts.database.pool().clone();

        std::thread::Builder::new()
            .name("postio-sync".to_string())
            .spawn(move || run(parts, pool, inbox))
            .map_err(|error| EngineError::new(format!("the sync engine did not start: {error}")))?;

        Ok(Engine { jobs })
    }

    /// Send everything the queue is holding for `account`.
    ///
    /// One pass. The caller decides when the next one runs — after a
    /// reconnect, after the shortest deferred backoff elapses, or when the
    /// user does something new.
    pub async fn drain(&self) -> Result<DrainSummary, EngineError> {
        self.ask(|reply| Job::Drain { reply }).await
    }

    /// Try a blocked link again, because the user supplied a new password.
    ///
    /// The one thing that clears [`Link::Blocked`]: a refused password does
    /// not get better on a timer, so nothing retries it until someone says
    /// the credentials have changed.
    pub async fn retry_now(&self) -> Result<Link, EngineError> {
        self.tell(|reply| Job::RetryNow { reply }).await
    }

    /// Tell the engine what the operating system says about the network.
    ///
    /// [`Link::Offline`] is deliberately not [`Link::Waiting`]: with no
    /// network there is nothing to retry against, so attempts are not spent
    /// and the status line says "offline" rather than counting down to a
    /// reconnection that cannot succeed.
    pub async fn set_network(&self, state: NetworkState) -> Result<Link, EngineError> {
        self.tell(|reply| Job::SetNetwork { state, reply }).await
    }

    /// What the link is doing right now.
    pub async fn link(&self) -> Result<Link, EngineError> {
        self.tell(|reply| Job::LinkState { reply }).await
    }

    /// What the backfill has done and has left to do.
    ///
    /// Every message that has entered the queue is in exactly one of these
    /// counts, which is what lets a progress display add up rather than
    /// drift.
    pub async fn backfill_progress(&self) -> Result<BackfillProgress, EngineError> {
        self.tell(|reply| Job::BackfillProgress { reply }).await
    }

    /// Bring `mailbox` in line with the server.
    ///
    /// The first pass enumerates it; every pass after that is incremental,
    /// falling back to a full re-enumeration when the server says the UID
    /// space it was counting on is gone.
    pub async fn sync(&self, mailbox: MailboxId) -> Result<SyncSummary, EngineError> {
        self.ask(|reply| Job::Sync { mailbox, reply }).await
    }

    /// Queue up to `limit` bodies worth having for `mailbox`.
    ///
    /// Called once per mailbox at startup and again whenever a sync finishes,
    /// which is when the set of messages missing a body has changed.
    pub async fn seed_backfill(
        &self,
        mailbox: MailboxId,
        limit: u32,
    ) -> Result<usize, EngineError> {
        self.ask(|reply| Job::SeedBackfill {
            mailbox,
            limit,
            reply,
        })
        .await
    }

    /// Ask for one message's body ahead of everything else.
    ///
    /// The reading pane opened it, so it is the one body the user is actually
    /// waiting for. `false` means there was nothing to fetch — the body is
    /// already here, or the message is gone.
    pub async fn request_body(&self, message: MessageId) -> Result<bool, EngineError> {
        self.ask(|reply| Job::RequestBody { message, reply }).await
    }

    /// As [`ask`](Self::ask), for a job whose answer cannot fail.
    async fn tell<T>(
        &self,
        job: impl FnOnce(tokio::sync::oneshot::Sender<T>) -> Job,
    ) -> Result<T, EngineError> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.jobs
            .send(job(reply))
            .await
            .map_err(|_| EngineError::new("the sync engine has stopped"))?;
        answer
            .await
            .map_err(|_| EngineError::new("the sync engine dropped the work"))
    }

    async fn ask<T>(
        &self,
        job: impl FnOnce(tokio::sync::oneshot::Sender<Result<T, EngineError>>) -> Job,
    ) -> Result<T, EngineError> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.jobs
            .send(job(reply))
            .await
            .map_err(|_| EngineError::new("the sync engine has stopped"))?;
        answer
            .await
            .unwrap_or_else(|_| Err(EngineError::new("the sync engine dropped the work")))
    }
}

/// How often the link is polled when nothing else is happening.
///
/// The supervisor is a state machine rather than a task, so the cadence is
/// the runtime's to choose. Five seconds is often enough that a dropped
/// connection is noticed promptly and rare enough that an idle Postio is
/// doing nothing most of the time — and it is only the *floor*: whatever hits
/// a broken connection tells the supervisor directly through `observe`, so
/// the common case does not wait for a tick at all.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// The engine's thread: a current-thread runtime and a connection of its own.
fn run(parts: EngineParts, pool: Pool, inbox: async_channel::Receiver<Job>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "the sync engine has no runtime");
            parts.events.emit(Event::Error {
                message: format!("the sync engine has no runtime: {error}"),
            });
            return;
        }
    };

    // One span around everything this thread does, so every line below can be
    // attributed to an account without every line having to say so. A sync run
    // spans a connection and many commands; flat lines cannot answer "which
    // run was this".
    let span = tracing::info_span!("engine", account = parts.account.get());
    runtime.block_on(tracing::Instrument::instrument(
        async move {
            let mut state = State {
                backfill: Backfill::new(parts.backfill),
                backfill_announced: None,
                supervisor: Supervisor::new(parts.reconnect),
                status: RefCell::new(StatusTracker::new()),
                online: false,
                to_sync: std::collections::VecDeque::new(),
                watcher: None,
                backfill_covered: false,
            };
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            // The first tick fires immediately; skipping it would leave the link
            // unexamined until the first interval elapsed.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            // A channel rather than a job, so that a bus which is slow to answer —
            // or absent — cannot hold up anything the user asked for. The listener
            // task lives on this same runtime and stops with it.
            let (network, mut network_changed) = tokio::sync::watch::channel(NetworkState::Unknown);
            if parts.network == NetworkSource::NetworkManager {
                tokio::spawn(crate::network::follow(network));
            }

            // Attempt the first connection right now, rather than waiting for
            // `ticker`'s first tick to win a race against whatever else is
            // ready: `postio-app::seed_the_backfill` sends a job the instant
            // `Engine::spawn` returns, so a job was already queued by the
            // time this loop ever ran for the first time, and `select!`
            // gives no guarantee about which of two simultaneously-ready
            // branches goes first. #109 measured that race costing a full
            // `POLL_INTERVAL` of connect latency on every single run.
            let moved = state
                .supervisor
                .poll(parts.backend.as_ref(), Utc::now(), entropy())
                .await;
            announce_link(&parts, &mut state, moved);
            handle_link_transition(&parts, &pool, &mut state).await;

            loop {
                tokio::select! {
                    job = inbox.recv() => match job {
                        Ok(job) => serve(job, &parts, &pool, &mut state).await,
                        // Every handle dropped: nothing more will be asked.
                        Err(_) => break,
                    },
                    _ = ticker.tick() => {
                        let moved = state
                            .supervisor
                            .poll(parts.backend.as_ref(), Utc::now(), entropy())
                            .await;
                        announce_link(&parts, &mut state, moved);
                    }
                    // The machine's own opinion of the network. It only ever moves
                    // the link between waiting and offline — the attempt count is
                    // the supervisor's, because NetworkManager knows about the
                    // interface and not about whether the server answers.
                    Ok(()) = network_changed.changed() => {
                        let reported = *network_changed.borrow_and_update();
                        let moved = state.supervisor.set_network(reported, Utc::now());
                        announce_link(&parts, &mut state, moved);
                    }
                }

                // Whatever the link just did, act on it before anything else: a
                // queue that has been waiting for a connection should go out the
                // moment there is one, not on the next thing the user happens to
                // do.
                handle_link_transition(&parts, &pool, &mut state).await;

                // A few mailboxes at a time, highest priority first, and the
                // inbox checked between waves: a folder with forty thousand
                // messages must not hold the engine away from a body the user
                // is waiting for. A wave in progress is cancelled the moment a
                // job arrives rather than waiting to be noticed here, which is
                // what keeps that true now that a wave is more than one
                // mailbox long. See `sync_wave`.
                while inbox.is_empty()
                    && state.supervisor.link().is_online()
                    && !state.to_sync.is_empty()
                {
                    sync_wave(&parts, &pool, &mut state, &inbox).await;
                }

                // Then fetch bodies, but only while nothing else is asking. One
                // at a time and the inbox checked between each, so the longest a
                // user waits behind the backfill is the one body already on the
                // wire.
                //
                // The queue is checked between each too, and for the same reason:
                // a body nobody has asked for is a guess about what the user will
                // want next, and a queued operation is something they have already
                // done. Speculation must not outrank it — a first sync of a large
                // mailbox is thousands of bodies long.
                while inbox.is_empty()
                    && state.supervisor.link().is_online()
                    && !has_queued_work(&parts, &pool)
                    && pump_body(&parts, &pool, &mut state).await
                {}

                // Then queue the next batch, and fetch that too. `seed` takes
                // the newest `seed_batch` messages still missing a body, and
                // for the life of the project nothing ever called it a second
                // time -- so the cap read as a horizon and everything below
                // the first batch of a folder waited to be opened (#318).
                //
                // Only when the queue has genuinely drained: `is_idle` is
                // false while the background lane is merely *paused* (metered,
                // or the user is doing something), and topping up then would
                // pull an entire archive into memory that policy has just said
                // not to fetch.
                while inbox.is_empty()
                    && state.supervisor.link().is_online()
                    && !has_queued_work(&parts, &pool)
                    && state.backfill.is_idle()
                    && top_up_backfill(&parts, &pool, &mut state) > 0
                {
                    while inbox.is_empty()
                        && state.supervisor.link().is_online()
                        && !has_queued_work(&parts, &pool)
                        && pump_body(&parts, &pool, &mut state).await
                    {}
                }

                // Then wait to be told about mail. This is the branch that idles,
                // so it goes last and races the inbox: a job arriving while an
                // `IDLE` is held must not wait out the timeout.
                if inbox.is_empty() && state.to_sync.is_empty() {
                    tokio::select! {
                        biased;
                        // Peeking rather than taking: whichever branch loses is
                        // cancelled, and a job taken off the queue by a branch
                        // that is then dropped is a job nobody serves.
                        _ = wait_for_job(&inbox) => {}
                        // A local mutation is not a job — nobody tells this thread
                        // that a row was written — and an `IDLE` is held for
                        // minutes. Without this branch, a flag set or a draft
                        // autosaved while connected would wait out the whole watch
                        // before going anywhere.
                        _ = wait_for_queued_work(&parts, &pool) => {}
                        _ = keep_watch(&parts, &mut state) => {}
                    }
                }
            }
        },
        span,
    ));
}

/// What the engine's thread keeps between jobs.
struct State {
    backfill: Backfill,
    /// When the body queue last said how far it had got.
    ///
    /// A status line redraws a few times a second at most, and a backfill
    /// against a fast server settles bodies far quicker than that, so this
    /// throttles the same way `StatusTracker` throttles a sync pass's
    /// batches and for the same reason. See [`announce_backfill`].
    backfill_announced: Option<std::time::Instant>,
    supervisor: Supervisor,
    /// The account's status line.
    ///
    /// Behind a `RefCell` because a sync wave runs several passes at once and
    /// every one of them folds its own batches in as they land — which is the
    /// whole of `postio-qhz.5` and cannot be deferred to the end of a pass.
    /// The engine is a single-thread runtime, so this is a borrow checker
    /// question rather than a thread-safety one, and no borrow is ever held
    /// across an await: every one of them is a single call on the tracker.
    status: RefCell<StatusTracker>,
    /// Whether the link was up last time anything looked.
    ///
    /// Only the *transition* is interesting: a connection that has been up
    /// for an hour is not a reason to drain again.
    online: bool,
    /// Mailboxes waiting for a sync pass.
    ///
    /// A queue rather than a loop, so one long mailbox cannot hold the engine
    /// away from everything else it is asked to do.
    to_sync: std::collections::VecDeque<MailboxId>,
    /// What is being watched for mail arriving, and how.
    ///
    /// `None` until the first connection tells us what the server can do —
    /// whether it offers `IDLE` decides between holding a connection open and
    /// asking on an interval, and guessing wrong either wastes a connection
    /// or misses mail.
    watcher: Option<Watcher>,
    /// Whether the last [`top_up_backfill`] found nothing left to queue.
    ///
    /// A latch, not a cache of the answer: it stops the engine re-asking
    /// every folder on every loop iteration once the account is covered, and
    /// is cleared by the two things that can make the answer change — a sync
    /// that wrote something, and the link coming up. Without it, an account
    /// whose remaining candidates are all set aside (over the size cap, or
    /// failed this session) would re-run one query per folder every five
    /// seconds for ever, finding the same nothing.
    backfill_covered: bool,
}

/// Whether the account's queue has anything due right now.
///
/// Deliberately silent about failure: a connection this cannot check out is
/// already being reported by whatever else wanted one, and a drain skipped for
/// a tick costs five seconds.
fn has_queued_work(parts: &EngineParts, pool: &Pool) -> bool {
    let Ok(connection) = pool.get() else {
        return false;
    };
    OperationQueueRepository::new(&connection)
        .pending(parts.account, Utc::now())
        .is_ok_and(|due| !due.is_empty())
}

/// Whether the link has come up since this was last asked.
fn came_up(state: &mut State) -> bool {
    let online = state.supervisor.link().is_online();
    let transition = online && !state.online;
    state.online = online;
    transition
}

/// Act on whatever the link just did, before anything else runs.
///
/// A queue that has been waiting for a connection should go out the moment
/// there is one, not on the next thing the user happens to do. Shared
/// between the loop and the one connection attempt made before it, so a
/// link that comes up before the loop's first iteration is handled exactly
/// the way one coming up mid-loop is. See the comment on that first attempt.
async fn handle_link_transition(parts: &EngineParts, pool: &Pool, state: &mut State) {
    if came_up(state) {
        // A session that failed bodies while the link was down set them
        // aside; a link that has just come up is exactly when they are worth
        // offering again, and when a folder that had nothing left may have
        // grown one.
        state.backfill_covered = false;
        let outcome = drain(parts, pool, state).await;
        announce_drain(&parts.events, parts.account, &outcome);
        // Before anything asks what is *in* a folder, find out which
        // folders there are. Everything below reads the local table, and on
        // a new account that table is empty until this runs.
        discover(parts, pool).await;
        // And find out what the server has been doing meanwhile.
        queue_every_mailbox(parts, pool, state);
        start_watching(parts, pool, state).await;
    } else if state.supervisor.link().is_online() && has_queued_work(parts, pool) {
        // The queue is filled by whoever performed the action — a flag, an
        // archive, a draft autosaved as it is typed — and none of them can
        // tell this thread that they wrote a row. So it asks, and the cost
        // of asking with an empty queue is one indexed read. Without this a
        // mutation made while connected would wait for the next
        // *reconnection* to go out, which on a machine that stays online is
        // never.
        let outcome = drain(parts, pool, state).await;
        announce_drain(&parts.events, parts.account, &outcome);
    }
}

/// Resolve once the inbox has something in it, without taking it.
///
/// `recv` would take the job and then be cancelled by the other branch of the
/// `select!`, losing it. This only ever observes.
async fn wait_for_job(inbox: &async_channel::Receiver<Job>) {
    while inbox.is_empty() && !inbox.is_closed() {
        tokio::time::sleep(WATCH_FLOOR).await;
    }
}

/// Resolve once the account's queue has something due, without draining it.
///
/// Polled rather than signalled because the queue is a table, written by
/// whichever thread performed the user's action, and SQLite has no way to say
/// so. Half a second is chosen against [`wait_for_job`], which already wakes
/// twenty times a second on the same loop: next to that, two indexed reads a
/// second cost nothing, and they are what makes an action taken while
/// connected reach the server while the user still remembers taking it.
async fn wait_for_queued_work(parts: &EngineParts, pool: &Pool) {
    loop {
        if has_queued_work(parts, pool) {
            return;
        }
        tokio::time::sleep(QUEUE_FLOOR).await;
    }
}

/// How often the queue is asked whether anything new is in it.
const QUEUE_FLOOR: Duration = Duration::from_millis(500);

/// The shortest a watch step will ever sleep.
///
/// Stops a watcher that says "nothing due, now" from spinning the loop.
const WATCH_FLOOR: Duration = Duration::from_millis(50);

/// Start watching for mail, now that there is a connection to watch over.
///
/// The inbox gets the dedicated connection — it is the one mailbox worth one
/// — and everything else is checked on an interval over the shared one.
#[tracing::instrument(skip_all)]
async fn start_watching(parts: &EngineParts, pool: &Pool, state: &mut State) {
    let capabilities = match parts.backend.capabilities().await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            tracing::warn!(%error, "cannot read the server's capabilities; not watching");
            return;
        }
    };
    // The names themselves: which of QRESYNC, CONDSTORE and SPECIAL-USE the
    // server offers is the first thing anyone asks when a resync misbehaves,
    // and a capability list is the server's public advertisement, not the
    // user's data.
    tracing::debug!(
        capabilities = ?capabilities.names(),
        incremental = capabilities.supports_incremental_sync(),
        "server capabilities"
    );
    let mut watcher = Watcher::new(parts.watch, &capabilities);

    let Ok(connection) = pool.get() else {
        tracing::warn!("no connection to read folders with; not watching");
        return;
    };
    let mailboxes = match MailboxRepository::new(&connection).list_for_account(parts.account) {
        Ok(mailboxes) => mailboxes,
        Err(error) => {
            tracing::error!(%error, "cannot read the account's folders; not watching");
            return;
        }
    };
    let mut watching = 0;
    for mailbox in mailboxes.into_iter().filter(|mailbox| mailbox.selectable) {
        watching += 1;
        let attention = if mailbox.role == postio_model::MailboxRole::Inbox {
            Attention::Push
        } else {
            Attention::Poll
        };
        watcher.watch(mailbox.id, mailbox.path.clone(), attention);
    }
    tracing::info!(watching, "watching for new mail");
    state.watcher = Some(watcher);
}

/// Do whatever the watcher says is due, and queue a sync if anything moved.
///
/// Returns when there is something to report or the step ends. The caller
/// races this against the inbox, so an `IDLE` being held never delays a job.
async fn keep_watch(parts: &EngineParts, state: &mut State) {
    if !state.supervisor.link().is_online() {
        // Nothing to watch over. Wait rather than spin.
        //
        // Said every tick, at `debug`: "the app is just sitting there" is a
        // real report, and the reason it is sitting there is the answer. A
        // `Blocked` link in particular never retries on its own — retrying a
        // refused credential is how an account gets locked — so without this
        // the silence is indistinguishable from a hang.
        tracing::debug!(
            link = %postio_model::address::redact_addresses(&format!("{:?}", state.supervisor.link())),
            "not connected; nothing to watch"
        );
        tokio::time::sleep(POLL_INTERVAL).await;
        return;
    }
    let Some(watcher) = state.watcher.as_mut() else {
        tokio::time::sleep(POLL_INTERVAL).await;
        return;
    };

    let now = Utc::now();
    // Push first: the inbox is the one mailbox worth a connection of its
    // own, and if it has nothing due the shared connection can do a round.
    let step = match watcher.next_push(now) {
        Watch::Wait { .. } => watcher.next_poll(now),
        step => step,
    };

    // Kept before the step is consumed: matching on it moves the path out.
    let watched = step_mailbox(&step);
    let wake = match step {
        Watch::Idle {
            mailbox,
            path,
            timeout,
            cancel,
        } => match parts.backend.idle(&path, timeout, &cancel).await {
            Ok(events) => watcher.woke(mailbox, &events, Utc::now()),
            Err(error) => {
                watcher.failed(mailbox, Utc::now());
                let moved = state.supervisor.observe(&error, Utc::now());
                announce_link(parts, state, moved);
                return;
            }
        },
        Watch::Poll { mailbox, path } => match parts.backend.status(&path).await {
            Ok(status) => watcher.observed(mailbox, &status, Utc::now()),
            Err(error) => {
                watcher.failed(mailbox, Utc::now());
                let moved = state.supervisor.observe(&error, Utc::now());
                announce_link(parts, state, moved);
                return;
            }
        },
        // Nothing due yet. Sleep until something might be, rather than
        // spinning the loop on a watcher with nothing to say.
        Watch::Wait { until } => {
            let delay = until
                .map(|at| (at - Utc::now()).to_std().unwrap_or_default())
                .unwrap_or(POLL_INTERVAL);
            tokio::time::sleep(delay.clamp(WATCH_FLOOR, POLL_INTERVAL)).await;
            return;
        }
    };

    if let (Wake::Changed, Some(mailbox)) = (wake, watched) {
        // What moved is not worth inferring from the events — they say only
        // *that* the mailbox moved — so pull.
        if !state.to_sync.contains(&mailbox) {
            state.to_sync.push_back(mailbox);
        }
    }
}

/// Which mailbox a step was about.
fn step_mailbox(step: &Watch) -> Option<MailboxId> {
    match step {
        Watch::Idle { mailbox, .. } | Watch::Poll { mailbox, .. } => Some(*mailbox),
        _ => None,
    }
}

/// Ask the server what folders the account has, and write them down.
///
/// Runs on every reconnection rather than only on the first, because folders
/// are created, renamed and removed by other clients — and because the first
/// connection of a brand-new account is the one where the local table is empty
/// and *everything* below depends on it.
///
/// A failure is reported and not fatal: the folders already known still sync.
/// A brand-new account with no folders yet is simply empty for another
/// reconnection, which is what it already looks like.
async fn discover(parts: &EngineParts, pool: &Pool) {
    let connection = match pool.get() {
        Ok(connection) => connection,
        Err(error) => {
            parts.events.emit(Event::Error {
                message: error.to_string(),
            });
            return;
        }
    };

    match discover::discover(
        &connection,
        parts.backend.as_ref(),
        parts.account,
        &parts.mailbox_roles,
    )
    .await
    {
        Ok(report) => {
            if report.changed() {
                // The sidebar reads the folder table, and nothing else is
                // going to tell it that the table just gained every folder
                // the account has.
                parts.events.emit(Event::MailboxesChanged {
                    account: parts.account,
                });
            }
        }
        Err(error) => {
            parts.events.emit(Event::Error {
                message: error.to_string(),
            });
        }
    }
}

/// Queue the next batch of bodies, INBOX first, and say how many.
///
/// # Why a top-up exists at all
///
/// `backfill::seed` asks for the newest `seed_batch` messages of one folder
/// that are still missing a body. That is a batch, and it was being used as a
/// horizon: `postio-app` seeded every folder once at startup and nothing in
/// the workspace ever called it again, so when those drained the background
/// lane had nothing to do for the rest of the process. Every message below
/// the first batch of its folder waited to be opened, and paid a round trip
/// when it was (#318).
///
/// Nothing here remembers how far it has got, and nothing needs to: a body
/// that lands sets `body_state` to `full`, which is what takes it out of
/// `needing_backfill`. So each call naturally asks for the *next* batch, and
/// the walk backwards through a folder is a property of the durable row
/// rather than of a cursor that a restart would lose.
///
/// # Why it stops
///
/// Two kinds of message stay in `needing_backfill` however often they are
/// offered: one over `max_body_bytes`, and one whose fetch failed this
/// session. `Backfill::enqueue` sets both aside and answers "queued nothing",
/// which is what this counts — so a folder whose only remaining candidates
/// are those reports zero and the caller stops, rather than re-queueing a
/// failing message in a tight loop against the server. `backfill_covered`
/// then keeps the engine from re-asking every folder on every loop iteration
/// until a sync or a reconnection says the answer may have changed.
///
/// INBOX first, by [`postio_sync::order::sync_priority`], for the reason
/// [`queue_every_mailbox`] gives: the folder the user is reading is the one
/// whose bodies are worth having first. It returns as soon as one folder
/// yields work, so a large Archive never gets ahead of it.
fn top_up_backfill(parts: &EngineParts, pool: &Pool, state: &mut State) -> usize {
    if state.backfill_covered {
        return 0;
    }
    let Ok(connection) = pool.get() else {
        // Nothing to read folders with. Not latched: the next pass will have
        // a connection, and latching here would stop the backfill for the
        // life of the process over one busy moment.
        return 0;
    };
    let mut mailboxes = match MailboxRepository::new(&connection).list_for_account(parts.account) {
        Ok(mailboxes) => mailboxes,
        Err(error) => {
            tracing::warn!(%error, "cannot read the account's folders to top up the backfill");
            return 0;
        }
    };
    mailboxes.sort_by_key(|mailbox| postio_sync::order::sync_priority(mailbox.role));

    for mailbox in mailboxes.iter().filter(|mailbox| mailbox.selectable) {
        let queued = match backfill::seed(
            &connection,
            &mut state.backfill,
            mailbox.id,
            parts.backfill.seed_batch,
        ) {
            Ok(queued) => queued,
            Err(error) => {
                tracing::warn!(%error, "cannot top up the backfill for a folder");
                continue;
            }
        };
        if queued > 0 {
            // Counts and a folder id, which is all a log may carry about mail.
            tracing::debug!(mailbox = mailbox.id.get(), queued, "backfill topped up");
            announce_backfill(parts, state, std::time::Instant::now());
            return queued;
        }
    }

    // Every folder had nothing left to offer. Stop asking until something
    // changes.
    state.backfill_covered = true;
    tracing::debug!("every folder's bodies are local, or set aside");
    0
}

/// Line every selectable folder up for a sync pass, INBOX first.
///
/// Sorted by [`postio_sync::order::sync_priority`], not by id and not by the
/// order discovery happened to list them in — a large Archive queued ahead of
/// INBOX left the user watching an empty inbox for as long as the archive took
/// (`postio-0d9.6`). The one-mailbox-at-a-time drain loop above this function
/// already finishes a mailbox before starting the next, so putting INBOX
/// first here is also what keeps it readable while everything behind it is
/// still syncing.
fn queue_every_mailbox(parts: &EngineParts, pool: &Pool, state: &mut State) {
    let Ok(connection) = pool.get() else {
        tracing::warn!("no connection to read folders with; syncing nothing this pass");
        return;
    };
    let mut mailboxes = match MailboxRepository::new(&connection).list_for_account(parts.account) {
        Ok(mailboxes) => mailboxes,
        Err(error) => {
            tracing::error!(%error, "cannot read the account's folders; syncing nothing");
            return;
        }
    };
    mailboxes.sort_by_key(|mailbox| postio_sync::order::sync_priority(mailbox.role));
    state.to_sync.clear();
    state.to_sync.extend(
        mailboxes
            .iter()
            .filter(|mailbox| mailbox.selectable)
            .map(|mailbox| mailbox.id),
    );
    // The one line that answers "the account connected and nothing happened".
    // Every folder-enumerating path here reads the *local* table; `discover`
    // is what fills it from the server on link-up, and a count of zero here
    // means that has not happened yet or found nothing.
    tracing::info!(
        known = mailboxes.len(),
        queued = state.to_sync.len(),
        "folders known locally, queued for sync"
    );
    if mailboxes.is_empty() {
        tracing::warn!(
            "no folders are known locally, so there is nothing to sync; either \
             discovery has not run yet or the server listed none"
        );
    }
}

/// How often a sync in progress tells the list that its mailbox has moved.
///
/// Each notification costs the list an invalidation and a page read, so one
/// per committed batch would be three hundred reloads across a first sync of
/// a large mailbox — the interaction budget spent entirely on repainting,
/// during the one stretch where the application most needs to feel alive.
/// Half a second is far below what reads as a delay and far above what reads
/// as a flicker.
const REPAINT_INTERVAL: Duration = Duration::from_millis(500);

/// Tells the list a mailbox has moved, at most every [`REPAINT_INTERVAL`].
///
/// The *first* batch is announced immediately and every later one is
/// throttled. That ordering is the whole point: the first screenful is what
/// the user is waiting for, and making them wait half a second for rows that
/// are already committed would be the same bug in miniature.
struct Repaint {
    events: EventSink,
    account: AccountId,
    mailbox: MailboxId,
    last: Option<Instant>,
}

impl Repaint {
    fn new(events: EventSink, account: AccountId, mailbox: MailboxId) -> Self {
        Repaint {
            events,
            account,
            mailbox,
            last: None,
        }
    }

    /// A batch reached the database. Announce it if it is time to.
    ///
    /// Only a *full enumeration* ever gets here: an incremental pull writes
    /// its rows without reporting [`Progress`], so the flicker of issue #72
    /// was never this. It is worth saying because the event this emits is
    /// `MessageListChanged`, which the view answers by discarding every
    /// visible row widget — the right answer while the list is being built
    /// out of nothing, and the wrong one for a delivery. If an incremental
    /// path ever starts reporting progress, it must not come through here.
    fn batch_committed(&mut self, now: Instant) {
        if !self.due(now) {
            return;
        }
        self.last = Some(now);
        self.events.emit(Event::MessageListChanged {
            account: self.account,
            mailbox: self.mailbox,
        });
    }

    fn due(&self, now: Instant) -> bool {
        match self.last {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= REPAINT_INTERVAL,
        }
    }
}

/// What one committed batch tells the rest of the application.
///
/// Both halves have to happen *as the batch lands*, and for the same reason:
/// a first sync of a large mailbox takes minutes, and everything the user can
/// see of it happens in here. Collecting the reports and folding them in after
/// the pass returned left the list empty (`postio-qhz.7`) and the status line
/// silent (`postio-qhz.5`) for the whole of it.
///
/// # Two throttles, deliberately
///
/// They are not the same number because they are not the same cost. A repaint
/// costs the list an invalidation and a page read, so it is held to
/// [`REPAINT_INTERVAL`]; a progress report costs a label, and
/// [`StatusTracker`] holds it to its own, shorter interval. The tracker also
/// never drops the report that completes a pass, which the list does not need
/// — a repaint that is superseded loses nothing, whereas a status line stuck
/// at 89% on a folder that has finished is simply wrong.
///
/// [`StatusTracker`]: postio_sync::StatusTracker
struct Committed<'a> {
    parts: &'a EngineParts,
    /// Borrowed rather than owned: it is the account's tracker and it outlives
    /// this pass. Holding it here is what lets the callback fold progress in
    /// as it arrives, which is the whole of `postio-qhz.5` — the previous
    /// arrangement could not, because the callback is `FnMut` and something
    /// else already had the `&mut`.
    ///
    /// A cell rather than a `&mut` because the passes of one sync wave share
    /// the tracker. The borrow below is taken and given back inside `batch`,
    /// never across an await.
    status: &'a RefCell<StatusTracker>,
    repaint: Repaint,
}

impl Committed<'_> {
    /// A batch reached the database.
    fn batch(&mut self, progress: Progress) {
        self.repaint.batch_committed(Instant::now());
        let reported = self.status.borrow_mut().on_progress(progress, Utc::now());
        if let Some(status) = reported {
            // Counts and an outcome, which is all a log may carry about mail.
            // Here because the two `sync started` / `sync finished` lines say
            // nothing about the minutes in between, and this is the line that
            // tells a pass that is working from one that has stalled.
            tracing::debug!(
                done = progress.fetched,
                total = progress.target,
                "sync progress announced"
            );
            announce_status(self.parts, &status);
        }
    }
}

/// Fetch one queued body, if there is one and it is worth doing now.
///
/// Returns whether it did anything, so the caller can keep going until the
/// queue is empty. One body per call on purpose: `Backfill::next_body` hands
/// out one at a time and holds it until `finished` reports it, which is what
/// bounds how long anything else can be stuck behind the backfill.
async fn pump_body(parts: &EngineParts, pool: &Pool, state: &mut State) -> bool {
    let Some(claim) = state.backfill.next_body() else {
        return false;
    };
    let message = claim.request.message;

    let connection = match pool.get() {
        Ok(connection) => connection,
        Err(error) => {
            // Report it settled rather than leaving it in flight for ever:
            // a claim nobody finishes is a queue that never drains.
            state.backfill.finished(
                message,
                Outcome::Failed {
                    reason: error.to_string(),
                },
            );
            return false;
        }
    };

    let outcome = backfill::fetch_body(
        &connection,
        &parts.blobs,
        parts.backend.as_ref(),
        &claim.request,
        &claim.cancel,
    )
    .await
    .unwrap_or_else(|error| {
        if let SyncError::Backend(backend) = &error {
            let moved = state.supervisor.observe(backend, Utc::now());
            announce_link(parts, state, moved);
        }
        Outcome::Failed {
            reason: error.to_string(),
        }
    });

    // The reading pane is waiting on exactly this for whatever the user just
    // opened, so it is worth saying the moment the bytes are local.
    if matches!(outcome, Outcome::Stored { .. }) {
        parts.events.emit(Event::BodyLoaded {
            account: parts.account,
            message,
        });
    }
    state.backfill.finished(message, outcome);
    announce_backfill(parts, state, std::time::Instant::now());
    true
}

/// Say how far the body queue has got.
///
/// Pushed rather than waited for. `Engine::backfill_progress` has always been
/// able to answer this and nothing ever asked, so the longest phase of a
/// first sync — the bodies, not the list — was drawn as `idle`: not merely
/// unreported but reported as nothing happening, which reads as stuck
/// (issue #74).
///
/// # Throttled, because a body is not a redraw
///
/// Called after every settled body, and against a fast server that is far
/// more often than a status line can redraw for. `StatusTracker` throttles a
/// sync pass's batches for exactly this reason and this follows its policy:
/// the same 250 ms floor, and the same rule that the update which *finishes*
/// the work is never dropped — an account is not allowed to sit at
/// `bodies 1994 of 2000` because the last body landed inside the window.
///
/// Emitting per body without this was measurably not free: it turned
/// `sync_wave.rs`'s INBOX-first test red about half the time, because the
/// events a 2000-message archive produced crowded out the work the engine
/// was supposed to be doing.
///
/// The other update never dropped is the first one after the queue was
/// empty, which is seeding: that is when the denominator becomes known, and
/// a line that appears only once a fetch has completed is silent for exactly
/// the stretch someone is most likely to be watching it.
///
/// `now` is a parameter rather than a read of the clock, for the same reason
/// [`Repaint::batch_committed`] and [`StatusTracker::on_progress`] take one:
/// a throttle a test cannot step through can only be tested by sleeping —
/// see engine.rs's own unit tests below, and issue #316's second cause,
/// which a real-time integration test could reproduce only by accident of
/// how fast the machine running it happened to be.
fn announce_backfill(parts: &EngineParts, state: &mut State, now: std::time::Instant) {
    let (done, total, drained) = backfill_snapshot(state);

    let opening = state.backfill_announced.is_none();
    let due = state
        .backfill_announced
        .is_none_or(|last| now.duration_since(last) >= BACKFILL_ANNOUNCE_INTERVAL);

    if !(drained || opening || due) {
        return;
    }
    emit_backfill_progress(parts, state, done, total, drained, now);
}

/// How often [`announce_backfill`] is allowed to report a settled body, at
/// most. [`StatusTracker`] holds its own progress line to the same floor,
/// for the same reason.
const BACKFILL_ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Announces the backfill's current progress unconditionally — never
/// throttled.
///
/// Issue #316's third cause: [`Job::RequestBody`] called the same throttled
/// [`announce_backfill`] a background body settling does, so whether the
/// reading pane's own open was ever reported at all depended on where the
/// 250 ms window happened to be. The floor above exists to protect the
/// engine from the firehose of events a fast archive produces (see
/// [`announce_backfill`]'s doc comment) — not from a person opening a
/// second message a moment after the first. `backfill.rs`'s own module doc
/// puts it as "the interactive lane always wins"; the status line honours
/// the same rule by never making a click wait out a background pass's
/// throttle window.
fn announce_backfill_now(parts: &EngineParts, state: &mut State, now: std::time::Instant) {
    let (done, total, drained) = backfill_snapshot(state);
    emit_backfill_progress(parts, state, done, total, drained, now);
}

/// `(done, total, drained)` as of right now — the read [`announce_backfill`]
/// and [`announce_backfill_now`] both act on.
fn backfill_snapshot(state: &State) -> (usize, usize, bool) {
    let progress = state.backfill.progress();
    let done = progress.settled();
    let total = done + progress.pending + progress.in_flight;
    let drained = progress.pending == 0 && progress.in_flight == 0;
    (done, total, drained)
}

/// Updates the throttle clock and emits the report both announce functions
/// share.
fn emit_backfill_progress(
    parts: &EngineParts,
    state: &mut State,
    done: usize,
    total: usize,
    drained: bool,
    now: std::time::Instant,
) {
    // Forget the clock once the queue is empty, so the next backfill's first
    // report is prompt rather than waiting out a window that started during
    // the last one.
    state.backfill_announced = if drained { None } else { Some(now) };

    parts.events.emit(Event::BackfillProgress {
        account: parts.account,
        done: done as u32,
        total: total as u32,
    });
}

async fn serve(job: Job, parts: &EngineParts, pool: &Pool, state: &mut State) {
    match job {
        Job::Drain { reply } => {
            let outcome = drain(parts, pool, state).await;
            announce_drain(&parts.events, parts.account, &outcome);
            let _ = reply.send(outcome);
        }
        Job::SeedBackfill {
            mailbox,
            limit,
            reply,
        } => {
            let outcome = with_connection(pool, |connection| {
                backfill::seed(connection, &mut state.backfill, mailbox, limit)
                    .map_err(|error| EngineError::new(error.to_string()))
            });
            // Say it now, not after the first body lands. Seeding is when the
            // denominator becomes known, and a status line that appears only
            // once a fetch has completed is silent for exactly the stretch
            // the user is most likely to be watching it.
            announce_backfill(parts, state, std::time::Instant::now());
            let _ = reply.send(outcome);
        }
        Job::RequestBody { message, reply } => {
            let outcome = with_connection(pool, |connection| {
                backfill::request_body(connection, &mut state.backfill, message)
                    .map_err(|error| EngineError::new(error.to_string()))
            });
            // Unconditional, not the throttled `announce_backfill` above:
            // the interactive lane always wins (#316).
            announce_backfill_now(parts, state, std::time::Instant::now());
            let _ = reply.send(outcome);
        }
        Job::RetryNow { reply } => {
            let moved = state.supervisor.retry_now(Utc::now());
            announce_link(parts, state, moved);
            let _ = reply.send(state.supervisor.link().clone());
        }
        Job::SetNetwork {
            state: network,
            reply,
        } => {
            let moved = state.supervisor.set_network(network, Utc::now());
            announce_link(parts, state, moved);
            let _ = reply.send(state.supervisor.link().clone());
        }
        Job::LinkState { reply } => {
            let _ = reply.send(state.supervisor.link().clone());
        }
        Job::BackfillProgress { reply } => {
            let _ = reply.send(state.backfill.progress());
        }
        Job::Sync { mailbox, reply } => {
            let outcome = sync(parts, pool, state, mailbox).await;
            let _ = reply.send(outcome);
        }
    }
}

/// One drain pass, with SMTP wired in so `Operation::Send` can actually send.
#[tracing::instrument(skip_all)]
async fn drain(
    parts: &EngineParts,
    pool: &Pool,
    state: &mut State,
) -> Result<DrainSummary, EngineError> {
    // A drain needs a session. Without one the queue is not *failed*, it is
    // simply not sent yet — which is the whole local-first promise: the write
    // already happened here, and reaching the server is a separate thing that
    // can wait. So a link that is not up leaves every row where it is.
    if !state.supervisor.link().is_online() {
        let moved = state
            .supervisor
            .poll(parts.backend.as_ref(), Utc::now(), entropy())
            .await;
        announce_link(parts, state, moved);
    }
    if !state.supervisor.link().is_online() {
        return Err(EngineError::new(offline_reason(state.supervisor.link())));
    }

    let connection = pool
        .get()
        .map_err(|error| EngineError::new(error.to_string()))?;

    let smtp = SmtpContext {
        connector: parts.smtp.as_ref(),
        secrets: parts.secrets.as_ref(),
        blobs: &parts.blobs,
    };
    let drainer = Drainer::with_policy(parts.backend.as_ref(), parts.retry).with_smtp(smtp);

    let report = match drainer.drain(&connection, parts.account, Utc::now()).await {
        Ok(report) => report,
        Err(error) => {
            tracing::warn!(%error, "the drain pass failed");
            // Noticed by the operation that hit it rather than by the next
            // tick: a session that died mid-drain has already cost the user
            // one action, and waiting five seconds to admit it costs another.
            if let SyncError::Backend(backend) = &error {
                let moved = state.supervisor.observe(backend, Utc::now());
                announce_link(parts, state, moved);
            }
            return Err(EngineError::new(error.to_string()));
        }
    };

    tracing::debug!(
        applied = report.applied,
        coalesced = report.coalesced,
        obsolete = report.obsolete,
        deferred = report.deferred,
        failed = report.failed.len(),
        needs_resync = report.needs_resync.len(),
        "drained the operation queue"
    );

    Ok(DrainSummary {
        applied: report.applied,
        coalesced: report.coalesced,
        obsolete: report.obsolete,
        deferred: report.deferred,
        failed: report
            .failed
            .iter()
            .map(|failure| failure.reason.clone())
            .collect(),
        needs_resync: report.needs_resync.clone(),
    })
}

/// How many mailboxes the engine will sync at the same time.
///
/// Bounded by the *database* pool rather than the IMAP one, because that is
/// the scarcer of the two here: each concurrent pass holds one SQLite
/// connection for as long as it runs, and the UI thread reads through the
/// same pool. Leaving [`RESERVED_FOR_ELSEWHERE`] behind is what keeps the
/// message list answering while a first sync is working through an archive.
///
/// The ceiling is separate and lower: the IMAP pool defaults to four
/// connections with one lane spoken for by `IDLE`, so a fourth concurrent
/// mailbox would not get a connection of its own — it would take turns on
/// somebody else's and pay a `SELECT` on every batch for the privilege (see
/// `postio_imap::imap::selection`). More lanes than the server will give
/// connections is slower than fewer.
const MAX_SYNC_LANES: usize = 3;

/// Database connections a sync wave will not touch.
///
/// One for the UI thread's reads — the message list, the reader, the sidebar
/// — and one for the engine's own housekeeping between waves.
const RESERVED_FOR_ELSEWHERE: usize = 2;

/// How many mailboxes may be in flight at once, for this pool.
fn sync_lanes(pool: &Pool) -> usize {
    pool.max_connections()
        .saturating_sub(RESERVED_FOR_ELSEWHERE)
        .clamp(1, MAX_SYNC_LANES)
}

/// Sync several mailboxes at once, in priority order, until a job arrives.
///
/// # Why concurrently
///
/// A first sync of a real account is the slowest thing Postio does, and
/// almost all of that wall clock is a round trip: measured against a live
/// server, a batch of two hundred headers spent an order of magnitude longer
/// waiting for the server than writing to SQLite. One mailbox at a time
/// leaves every one of those waits unused. Running two or three passes
/// together overlaps them, and costs nothing but connections the pool was
/// already sized for (`postio-0d9.7`).
///
/// # Why INBOX still comes first
///
/// [`queue_every_mailbox`] sorts by [`postio_sync::order::sync_priority`] and
/// this takes from the front, so INBOX is in the *first* wave rather than
/// behind an archive — which is the whole of `postio-0d9.6`, and matters more
/// now rather than less: a lane occupied by a forty-thousand-message archive
/// is a lane INBOX is not waiting behind.
///
/// # Why the connections are all taken before any pass starts
///
/// [`postio_storage::db::Pool::get`] blocks the *OS thread* on a condvar when
/// the pool is exhausted, and the engine is a single-thread runtime. Two
/// concurrent passes both calling it with nothing left to hand out would both
/// block that one thread, with nothing able to run and release a connection:
/// a real deadlock rather than contention. Acquiring every lane's connection
/// up front, sequentially, before anything is concurrent is what makes that
/// unreachable — see `docs/engineering-notes.md`.
async fn sync_wave(
    parts: &EngineParts,
    pool: &Pool,
    state: &mut State,
    inbox: &async_channel::Receiver<Job>,
) {
    if !state.supervisor.link().is_online() {
        let moved = state
            .supervisor
            .poll(parts.backend.as_ref(), Utc::now(), entropy())
            .await;
        announce_link(parts, state, moved);
    }
    if !state.supervisor.link().is_online() {
        return;
    }

    // Every connection this wave will ever need, taken one at a time while
    // nothing else on this thread is running. See the note above.
    let mut lanes: Vec<(MailboxId, PooledConnection)> = Vec::new();
    for _ in 0..sync_lanes(pool) {
        let Some(mailbox) = state.to_sync.pop_front() else {
            break;
        };
        match pool.get() {
            Ok(connection) => lanes.push((mailbox, connection)),
            Err(error) => {
                // Nothing to sync it with. Put it back rather than dropping
                // it: the next wave will have a connection.
                state.to_sync.push_front(mailbox);
                tracing::warn!(%error, "no connection for a sync lane");
                break;
            }
        }
    }
    if lanes.is_empty() {
        return;
    }

    // Shared by every pass in the wave, and cancelled as one: a job the user
    // is waiting on must not queue behind three mailboxes. A cancelled pass
    // keeps everything it committed and resumes from there, so this costs at
    // most the batch that was on the wire.
    let cancel = postio_imap::cancel::CancelToken::new();
    let mut outcomes: Vec<PassOutcome> = Vec::with_capacity(lanes.len());

    {
        // Borrowed by every pass at once, which is what the `RefCell` is for.
        // Nothing holds the borrow across an await: `Committed::batch` takes
        // it, folds one batch in, and gives it back.
        let status = &state.status;
        let mut running = FuturesUnordered::new();
        for (mailbox, connection) in &lanes {
            running.push(sync_pass(parts, connection, status, *mailbox, &cancel));
        }

        let mut asked_to_stop = false;
        while !running.is_empty() {
            tokio::select! {
                biased;
                // Completions first: a wave that is already finishing should
                // finish rather than notice the job it was about to be told
                // about and mark everything cancelled.
                Some(outcome) = running.next() => outcomes.push(outcome),
                _ = wait_for_job(inbox), if !asked_to_stop => {
                    asked_to_stop = true;
                    cancel.cancel();
                }
            }
        }
    }

    // Then fold every pass back into the engine's state, sequentially — this
    // is where the `&mut` work lives, and there is no reason for it to be
    // concurrent.
    let connection = &lanes[0].1;
    let mut interrupted: Vec<MailboxId> = Vec::new();
    for outcome in outcomes {
        let stopped_early = outcome.was_cancelled();
        if stopped_early {
            interrupted.push(outcome.mailbox);
        }
        // The wave is the engine acting on its own initiative, so a failed
        // pass is reported to the user here rather than returned to a caller
        // who asked for it. An interrupted one is not a failure: it stops
        // where it was told to and resumes, and saying so would put a
        // spurious error on screen every time the user did something during a
        // first sync.
        if let Err(error) = settle_pass(parts, state, connection, outcome)
            && !stopped_early
        {
            parts.events.emit(Event::Error {
                message: error.message().to_string(),
            });
        }
    }

    // Back on the front of the queue, in the order they were taken off it —
    // `outcomes` is in completion order, which says nothing about priority,
    // and INBOX must not come back behind the archive it started alongside.
    for (mailbox, _) in lanes.iter().rev() {
        if interrupted.contains(mailbox) {
            state.to_sync.push_front(*mailbox);
        }
    }
    if !interrupted.is_empty() {
        tracing::debug!(
            mailboxes = interrupted.len(),
            "sync wave interrupted; requeued"
        );
    }
}

/// What one pass did, before the engine's own state hears about it.
struct PassOutcome {
    mailbox: MailboxId,
    result: Result<SyncSummary, PassFailure>,
}

impl PassOutcome {
    /// Whether this pass stopped because the wave cancelled it, rather than
    /// finishing or failing. Its mailbox is owed another turn.
    fn was_cancelled(&self) -> bool {
        matches!(
            &self.result,
            Err(PassFailure::Failed(SyncError::Backend(
                BackendError::Cancelled
            )))
        )
    }
}

/// Why a pass did not produce a summary.
enum PassFailure {
    /// The folder was queued and then went away — renamed or removed by
    /// another client between discovery and this wave. Not the link's fault,
    /// so the supervisor never hears about it.
    NoSuchMailbox,
    /// The pass ran and failed.
    Failed(SyncError),
}

impl fmt::Display for PassFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchMailbox => f.write_str("that folder is not in the local store"),
            Self::Failed(error) => write!(f, "{error}"),
        }
    }
}

/// One sync pass over one mailbox.
///
/// `sync_mailbox` the first time and `resync_mailbox` afterwards, decided by
/// whether the mailbox has sync state — which is the record of a pass having
/// completed, and the thing `resync_mailbox` needs to be incremental against.
///
/// Deliberately touches nothing that needs `&mut State`: several of these run
/// at once (see [`sync_wave`]), and what they share — the status line — is
/// behind a `RefCell` that no pass holds across an await. Everything else is
/// [`settle_pass`]'s job, afterwards.
// Named `sync` rather than `sync_pass`: the span is the log surface for a
// mailbox pass and predates the split, and `logging_privacy` keys on it.
#[tracing::instrument(name = "sync", skip_all, fields(mailbox = mailbox.get(), path, incremental))]
async fn sync_pass(
    parts: &EngineParts,
    connection: &PooledConnection,
    status: &RefCell<StatusTracker>,
    mailbox: MailboxId,
    cancel: &postio_imap::cancel::CancelToken,
) -> PassOutcome {
    let failed = |failure: PassFailure| PassOutcome {
        mailbox,
        result: Err(failure),
    };
    let record = match MailboxRepository::new(connection).get(mailbox) {
        Ok(Some(record)) => record,
        Ok(None) => return failed(PassFailure::NoSuchMailbox),
        Err(error) => return failed(PassFailure::Failed(error.into())),
    };
    let synced_before = match SyncStateRepository::new(connection).get(mailbox) {
        Ok(state) => state.is_some(),
        Err(error) => return failed(PassFailure::Failed(error.into())),
    };

    // A folder *path* is a name the user chose for a container, not a message
    // and not an address; it is what makes a per-mailbox line legible at all.
    tracing::Span::current().record("path", record.path.as_str());
    tracing::Span::current().record("incremental", synced_before);
    tracing::info!("sync started");

    announce_status(parts, &status.borrow_mut().on_sync_started(mailbox));

    let mut committed = Committed {
        parts,
        status,
        repaint: Repaint::new(parts.events.clone(), parts.account, mailbox),
    };
    // Populated only by the incremental branch below: a first sync or a
    // rebuild can insert thousands of messages that are new to *this
    // store*, not new mail the user has not seen arrive — see
    // `Event::NewMail`'s doc comment and postio-du6's "no notification
    // storm" acceptance criterion.
    let mut arrived: Vec<MessageId> = Vec::new();
    // Whether `arrived` accounts for everything this pass did to the list.
    //
    // `SyncSummary` cannot answer this: it folds `changed + vanished` into
    // one `updated` count, and an incremental pull reports a brand-new
    // message as *changed* rather than inserted. The distinction only exists
    // here, while the `Outcome` is still whole — `arrived` is the subset of
    // `changed` that was not known before, so `changed == arrived.len()`
    // means nothing already on screen moved or changed under the user.
    let mut only_arrivals = false;
    let result = if synced_before {
        resync::resync_mailbox(
            connection,
            parts.backend.as_ref(),
            &record,
            cancel,
            |progress| committed.batch(progress),
        )
        .await
        .map(|outcome| {
            if let resync::Outcome::Incremental {
                arrived: ids,
                changed,
                vanished,
                ..
            } = &outcome
            {
                arrived = ids.clone();
                only_arrivals = *vanished == 0 && *changed == ids.len();
            }
            summarise_resync(outcome)
        })
    } else {
        initial::sync_mailbox(
            connection,
            parts.backend.as_ref(),
            &record,
            cancel,
            |progress| committed.batch(progress),
        )
        .await
        .map(|report| SyncSummary {
            inserted: report.inserted,
            updated: report.updated,
            threaded: report.threaded,
            full: true,
        })
    };

    if let Ok(summary) = &result {
        tracing::info!(
            inserted = summary.inserted,
            updated = summary.updated,
            threaded = summary.threaded,
            full = summary.full,
            "sync finished"
        );
        // `MessageListChanged` is the view's blunt instrument: it means the
        // *order* moved, and `MessageList::invalidate` answers it by telling
        // GTK every row was removed and re-added — every visible row widget
        // discarded and rebuilt, every page re-read, the scroll anchor and
        // the selection rediscovered. That is the right answer to a resort
        // and the wrong one to a delivery.
        //
        // When `arrived` accounts for every insert and nothing was updated
        // or re-threaded, the `NewMail` below says the same thing precisely,
        // and the view prepends instead. Emitting both means the blunt one
        // runs anyway and the cheap one never gets to matter — issue #72,
        // the flicker on every sync tick.
        let described_by_arrivals = only_arrivals && !arrived.is_empty();
        if summary.changed() && !described_by_arrivals {
            // The list showing this folder has to re-read it.
            parts.events.emit(Event::MessageListChanged {
                account: parts.account,
                mailbox,
            });
        }
        if !arrived.is_empty() {
            parts.events.emit(Event::NewMail {
                account: parts.account,
                mailbox,
                messages: arrived,
            });
        }
    }

    PassOutcome {
        mailbox,
        result: result.map_err(PassFailure::Failed),
    }
}

/// Fold a finished pass into the engine's state, and say what it did.
///
/// Split from [`sync_pass`] because everything in here needs `&mut State`,
/// and passes run concurrently. Sequential and cheap: no network, one
/// possible indexed read.
fn settle_pass(
    parts: &EngineParts,
    state: &mut State,
    connection: &PooledConnection,
    outcome: PassOutcome,
) -> Result<SyncSummary, EngineError> {
    let PassOutcome { mailbox, result } = outcome;
    let now = Utc::now();

    // A pass the wave cancelled is not a failure and not a completed pass: it
    // stopped where it was told to, keeping everything it had committed, and
    // whoever requeues it resumes from there. It leaves the status line
    // (nothing is fetching for it any more) and touches nothing else — in
    // particular not the supervisor, since the link is fine.
    if let Err(PassFailure::Failed(SyncError::Backend(BackendError::Cancelled))) = &result {
        announce_status(
            parts,
            &state.status.borrow_mut().on_sync_finished(mailbox, now),
        );
        return Err(EngineError::new("the sync pass was interrupted"));
    }

    match result {
        Ok(summary) => {
            announce_status(
                parts,
                &state.status.borrow_mut().on_sync_finished(mailbox, now),
            );
            if summary.changed() {
                // A sync is exactly when the set of messages missing a body
                // changed, so it is exactly when the backfill is worth
                // seeding again. It also means there is more to cover than
                // the last top-up found: new mail arrives *above* whatever
                // point the walk backwards through the archive had reached.
                state.backfill_covered = false;
                if let Err(error) = backfill::seed(
                    connection,
                    &mut state.backfill,
                    mailbox,
                    parts.backfill.seed_batch,
                ) {
                    parts.events.emit(Event::Error {
                        message: error.to_string(),
                    });
                }
            }
            Ok(summary)
        }
        Err(failure) => {
            tracing::warn!(error = %failure, "sync failed");
            if let PassFailure::Failed(SyncError::Backend(backend)) = &failure {
                let moved = state.supervisor.observe(backend, now);
                announce_link(parts, state, moved);
            }
            announce_status(
                parts,
                &state.status.borrow_mut().on_sync_finished(mailbox, now),
            );
            Err(EngineError::new(failure.to_string()))
        }
    }
}

/// One mailbox, synced now, for a caller that is waiting on the answer.
///
/// The wave above is how the engine syncs on its own initiative; this is
/// [`Job::Sync`] — the watcher saw mail arrive, or the user asked. It takes
/// one connection and runs one pass, so a single mailbox never pays for the
/// wave's machinery.
async fn sync(
    parts: &EngineParts,
    pool: &Pool,
    state: &mut State,
    mailbox: MailboxId,
) -> Result<SyncSummary, EngineError> {
    if !state.supervisor.link().is_online() {
        let moved = state
            .supervisor
            .poll(parts.backend.as_ref(), Utc::now(), entropy())
            .await;
        announce_link(parts, state, moved);
    }
    if !state.supervisor.link().is_online() {
        return Err(EngineError::new(offline_reason(state.supervisor.link())));
    }

    let connection = pool
        .get()
        .map_err(|error| EngineError::new(error.to_string()))?;
    let cancel = postio_imap::cancel::CancelToken::new();
    let outcome = sync_pass(parts, &connection, &state.status, mailbox, &cancel).await;
    settle_pass(parts, state, &connection, outcome)
}

/// What a resync did, in the engine's own terms.
fn summarise_resync(outcome: resync::Outcome) -> SyncSummary {
    match outcome {
        resync::Outcome::UpToDate => SyncSummary::default(),
        resync::Outcome::Full { report, .. } | resync::Outcome::Rebuilt { report } => SyncSummary {
            inserted: report.inserted,
            updated: report.updated,
            threaded: report.threaded,
            full: true,
        },
        // An incremental pull counts what moved rather than what it wrote:
        // a flag change is an update, and a message the server no longer has
        // is a row removed. Both mean the list showing this folder is stale.
        resync::Outcome::Incremental {
            changed, vanished, ..
        } => SyncSummary {
            inserted: 0,
            updated: changed + vanished,
            threaded: 0,
            full: false,
        },
    }
}

/// Why the link is not usable, phrased for the user.
fn offline_reason(link: &Link) -> String {
    match link {
        Link::Blocked(blocker) => blocker.reason().to_string(),
        Link::Offline => "there is no network".to_string(),
        Link::Waiting { .. } => "not connected yet".to_string(),
        Link::Online { .. } => "connected".to_string(),
    }
}

/// Say what the status line should show now.
///
/// One place, because a link change and a sync pass both move the same
/// status and the frontend should not have to tell which produced it.
/// `SyncProgress` rather than `ConnectionChanged` while a pass is counting:
/// they are different questions — *is there a connection* and *how far has
/// this got* — and the sidebar draws them differently.
fn announce_status(parts: &EngineParts, status: &SyncStatus) {
    if let SyncStatus::Syncing {
        progress: Some(progress),
        ..
    } = status
    {
        parts.events.emit(Event::SyncProgress {
            account: parts.account,
            done: progress.done,
            total: progress.total,
        });
        return;
    }
    parts.events.emit(Event::ConnectionChanged {
        account: parts.account,
        state: connection_of(status),
    });
    // The typed category rides on the state; the prose travels beside it,
    // which is what the status line reads.
    if let SyncStatus::Error { reason, .. } = status {
        parts.events.emit(Event::Error {
            message: reason.clone(),
        });
    }
}

/// The frontend's summary of where an account stands.
///
/// One function rather than two inline matches, because a link change and a
/// sync pass must agree — and because the `Failing` reason (ADR 0005 Q10) is
/// a mapping worth testing on its own. `needs_credentials` is how
/// `postio-sync` distinguishes its two blockers: a refused credential is
/// `Auth` (the user signs in again; the blocked link never retries on a
/// timer), and everything else a retry cannot fix is `Config` (the user
/// edits a setting). `Network` and `Server` are reserved: transient trouble
/// is `Connecting` while the backoff works, and a per-operation server
/// failure belongs to the operation queue, not the link.
fn connection_of(status: &SyncStatus) -> postio_core::ConnectionState {
    match status {
        SyncStatus::Offline => postio_core::ConnectionState::Offline,
        SyncStatus::Connecting => postio_core::ConnectionState::Connecting,
        SyncStatus::Idle { .. } | SyncStatus::Syncing { .. } => {
            postio_core::ConnectionState::Online
        }
        SyncStatus::Error {
            needs_credentials, ..
        } => postio_core::ConnectionState::Failing {
            reason: if *needs_credentials {
                postio_core::FailureReason::Auth
            } else {
                postio_core::FailureReason::Config
            },
        },
    }
}

/// Turn a link transition into what the status line shows.
///
/// `StatusTracker` already throttles and shapes these, so this is only the
/// last hop: its `SyncStatus` onto core's own `ConnectionState`, which is the
/// summary the frontend reads and which `postio-core` promised would not
/// change when the sync engine's state machine does.
fn announce_link(parts: &EngineParts, state: &mut State, moved: Option<Link>) {
    let Some(link) = moved else {
        return;
    };
    // Only transitions reach here, so this is one line per actual change
    // rather than one per poll.
    // Through `Debug`, then redacted: a `Blocked` link carries the reason it
    // is blocked, and an unusable credential names the account.
    tracing::info!(
        link = %postio_model::address::redact_addresses(&format!("{link:?}")),
        "connection state changed"
    );
    let status = state.status.borrow_mut().on_link(&link);
    parts.events.emit(Event::ConnectionChanged {
        account: parts.account,
        state: connection_of(&status),
    });
    // The typed category rides on the state; the prose travels beside it,
    // which is what the status line reads.
    if let SyncStatus::Error { reason, .. } = &status {
        tracing::error!(
            reason = %postio_model::address::redact_addresses(reason),
            "connection failing"
        );
        parts.events.emit(Event::Error {
            message: reason.clone(),
        });
    }
}

/// Say what a drain did, so the UI hears it the way it hears everything else.
fn announce_drain(
    events: &EventSink,
    account: AccountId,
    outcome: &Result<DrainSummary, EngineError>,
) {
    match outcome {
        Ok(summary) => {
            // A mailbox the server disagreed with has to be re-read, and the
            // list showing it is the thing that has to know.
            for mailbox in &summary.needs_resync {
                events.emit(Event::MessageListChanged {
                    account,
                    mailbox: *mailbox,
                });
            }
            // Never silently empty: an operation given up on is one the user
            // believes happened.
            for reason in &summary.failed {
                events.emit(Event::Error {
                    message: reason.clone(),
                });
            }
        }
        Err(error) => {
            events.emit(Event::Error {
                message: error.message().to_string(),
            });
        }
    }
}

/// Jitter for the reconnect backoff.
///
/// The clock rather than a random number generator: this only has to stop a
/// fleet of clients retrying in lockstep, and a dependency for that would be
/// a dependency for nothing.
fn entropy() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as u64)
        .unwrap_or(0)
}

/// Run `work` with a connection, turning a checkout failure into an error the
/// user could read.
fn with_connection<T>(
    pool: &Pool,
    work: impl FnOnce(&postio_storage::PooledConnection) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let connection = pool
        .get()
        .map_err(|error| EngineError::new(error.to_string()))?;
    work(&connection)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_refused_credential_fails_as_auth_and_anything_else_as_config() {
        // ADR 0005 Q10: the reason is what the *user* can do. postio-sync's
        // blocked link tells its two cases apart with `needs_credentials`,
        // and both must surface typed rather than as prose to parse.
        let auth = super::connection_of(&postio_sync::SyncStatus::Error {
            reason: "the server refused the password".into(),
            needs_credentials: true,
        });
        let config = super::connection_of(&postio_sync::SyncStatus::Error {
            reason: "certificate does not verify".into(),
            needs_credentials: false,
        });

        assert_eq!(
            auth,
            postio_core::ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth
            }
        );
        assert_eq!(
            config,
            postio_core::ConnectionState::Failing {
                reason: postio_core::FailureReason::Config
            }
        );
    }

    #[test]
    fn transient_trouble_is_connecting_not_failing() {
        // Backoff that will retry on its own is not a person-shaped problem.
        assert_eq!(
            super::connection_of(&postio_sync::SyncStatus::Connecting),
            postio_core::ConnectionState::Connecting
        );
    }
    use super::*;
    use postio_core::bridge::event_channel;

    /// A `State` with nothing queued, for tests that only care about
    /// [`queue_every_mailbox`]'s effect on `to_sync`.
    fn empty_state() -> State {
        State {
            backfill: Backfill::new(BackfillPolicy::default()),
            backfill_announced: None,
            supervisor: Supervisor::new(ReconnectPolicy::default()),
            status: RefCell::new(StatusTracker::new()),
            online: false,
            to_sync: std::collections::VecDeque::new(),
            watcher: None,
            backfill_covered: false,
        }
    }

    /// `EngineParts` over `database`, with the receiving end of its own
    /// event channel — kept, not discarded, so a test can see what got
    /// announced.
    fn parts_over(database: Database) -> (EngineParts, postio_core::bridge::EventStream) {
        let connection = database.connection().expect("checkout");
        let account = postio_storage::test_support::account(&connection);
        drop(connection);
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(directory.keep()).expect("a blob store");
        let (sink, events) = postio_core::bridge::event_channel();
        let parts = EngineParts {
            account: account.id,
            database,
            blobs,
            backend: Arc::new(postio_imap::backend::MockBackend::new()),
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            secrets: Arc::new(postio_imap::secret::MemorySecretStore::default()),
            events: sink,
            retry: RetryPolicy::default(),
            backfill: BackfillPolicy::default(),
            reconnect: ReconnectPolicy::default(),
            watch: WatchPolicy::default(),
            network: NetworkSource::Ignored,
            mailbox_roles: Default::default(),
        };
        (parts, events)
    }

    /// postio-0d9.6: a 37,699-message Archive was queued ahead of an
    /// untouched INBOX because nothing ordered the sync queue across
    /// mailboxes — it was whatever `list_for_account` happened to return.
    /// `queue_every_mailbox` must put INBOX first, then the folders a person
    /// reads next, then everything else, regardless of the order the
    /// mailboxes were created in.
    #[test]
    fn queueing_every_mailbox_puts_inbox_first_and_orders_the_rest_by_role() {
        let database = postio_storage::test_support::memory();
        let (parts, _events) = parts_over(database.clone());
        let connection = database.connection().expect("checkout");

        // Created deliberately out of role order, archive first, so a queue
        // built from creation or discovery order would fail this test.
        let archive =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Archive");
        let regular =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Projects");
        let trash =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Trash");
        let sent = postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Sent");
        let inbox =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts), "INBOX");
        let junk = postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Junk");
        let drafts =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Drafts");
        let flagged =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts), "Flagged");
        drop(connection);

        let mut state = empty_state();
        queue_every_mailbox(&parts, database.pool(), &mut state);

        let queued: Vec<MailboxId> = state.to_sync.into_iter().collect();
        assert_eq!(
            queued,
            vec![
                inbox.id, flagged.id, drafts.id, sent.id, archive.id, junk.id, trash.id,
                regular.id,
            ],
            "INBOX must sync first and a large Archive must never outrank it"
        );
    }

    fn account_of(parts: &EngineParts) -> postio_model::Account {
        let connection = parts.database.connection().expect("checkout");
        postio_storage::repository::AccountRepository::new(&connection)
            .get(parts.account)
            .expect("read the test account")
            .expect("the test account exists")
    }

    /// The throttle behind [`Repaint`], driven by a clock the test owns.
    ///
    /// Time is a parameter rather than a read, for the same reason
    /// `postio-search` takes `today`: a schedule you cannot step through is a
    /// schedule you can only test by sleeping.
    #[test]
    fn the_first_batch_is_announced_at_once_and_the_rest_are_throttled() {
        let (sink, events) = event_channel();
        let mut repaint = Repaint::new(sink, AccountId::new(1), MailboxId::new(4));
        let start = Instant::now();

        // The first screenful is what the user is waiting for.
        repaint.batch_committed(start);
        assert_eq!(events.len(), 1, "the first batch has to show immediately");

        // Everything inside the window folds into it. This is the case that
        // matters: three hundred batches of an initial sync must not be three
        // hundred reloads.
        for step in 1..=50 {
            // Spread across the window without ever reaching its end.
            let inside = REPAINT_INTERVAL / 100 * (step % 100);
            repaint.batch_committed(start + inside);
        }
        assert_eq!(events.len(), 1, "batches inside the window must coalesce");

        // And the next window opens.
        repaint.batch_committed(start + REPAINT_INTERVAL);
        assert_eq!(events.len(), 2);
        repaint.batch_committed(start + REPAINT_INTERVAL * 2);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn a_pass_that_commits_nothing_says_nothing() {
        // An up-to-date resync reports no batches at all, and a list that
        // reloaded anyway would be paying for a sync that changed nothing.
        let (sink, events) = event_channel();
        let _repaint = Repaint::new(sink, AccountId::new(1), MailboxId::new(4));

        assert_eq!(events.len(), 0);
    }

    // -- The backfill announce throttle (issue #316) -------------------

    /// A body request the backfill has never seen before, distinct per `n`
    /// so several can be claimed without colliding.
    fn body_request(n: u32) -> backfill::BodyRequest {
        backfill::BodyRequest {
            message: MessageId::new(n as i64),
            mailbox: MailboxId::new(1),
            path: "INBOX".to_string(),
            uid: postio_model::Uid::new(n),
            size: 100,
            received_at: Utc::now(),
        }
    }

    /// Documents the throttle `announce_backfill` still has, over an
    /// explicit clock rather than a real one — see its own doc comment for
    /// why a real-time version of this test would prove nothing reliably.
    #[test]
    fn a_second_announcement_inside_the_floor_is_throttled() {
        let database = postio_storage::test_support::memory();
        let (parts, events) = parts_over(database);
        let mut state = empty_state();
        let t0 = Instant::now();

        state.backfill.request_now(body_request(1));
        announce_backfill(&parts, &mut state, t0);
        assert_eq!(events.len(), 1, "the first claim must announce at once");

        state.backfill.request_now(body_request(2));
        announce_backfill(&parts, &mut state, t0 + Duration::from_millis(10));
        assert_eq!(
            events.len(),
            1,
            "10ms is well inside the 250ms floor -- a second background-\
             shaped announce this soon must still be throttled"
        );
    }

    /// Issue #316, cause 3: an interactive open must never wait out a
    /// background pass's throttle window the way the test above shows a
    /// second ordinary announce does.
    #[test]
    fn announce_backfill_now_is_never_throttled() {
        let database = postio_storage::test_support::memory();
        let (parts, events) = parts_over(database);
        let mut state = empty_state();
        let t0 = Instant::now();

        state.backfill.request_now(body_request(1));
        announce_backfill(&parts, &mut state, t0);
        assert_eq!(events.len(), 1);

        // A second interactive claim, 10ms later -- exactly the timing the
        // test above proves the throttled path swallows.
        state.backfill.request_now(body_request(2));
        announce_backfill_now(&parts, &mut state, t0 + Duration::from_millis(10));

        assert_eq!(
            events.len(),
            2,
            "an interactive open must be announced immediately, throttle \
             window or not"
        );
        match events.try_next() {
            Some(Event::BackfillProgress { done, total, .. }) => {
                assert_eq!((done, total), (0, 1), "the first announce")
            }
            other => panic!("expected the first BackfillProgress, got {other:?}"),
        }
        match events.try_next() {
            Some(Event::BackfillProgress { done, total, .. }) => {
                assert_eq!(
                    (done, total),
                    (0, 2),
                    "the second announce must reflect both claims"
                )
            }
            other => panic!("expected the second BackfillProgress, got {other:?}"),
        }
    }

    /// A drain always reports and resets the clock, whichever announce
    /// function reaches it — `emit_backfill_progress` is the one place both
    /// funnel through, and this is what stops a finished queue from sticking
    /// at `2000 of 2000` (the same trap `SyncProgress` avoids).
    #[test]
    fn a_drained_queue_resets_the_throttle_clock() {
        let database = postio_storage::test_support::memory();
        let (parts, events) = parts_over(database);
        let mut state = empty_state();
        let t0 = Instant::now();

        state.backfill.request_now(body_request(1));
        announce_backfill(&parts, &mut state, t0);
        let claim = state.backfill.next_body().expect("the claim just queued");
        state
            .backfill
            .finished(claim.request.message, Outcome::Gone);
        announce_backfill(&parts, &mut state, t0 + Duration::from_millis(10));
        assert_eq!(
            events.len(),
            2,
            "a drain must always be reported, throttle or not"
        );
        assert_eq!(
            state.backfill_announced, None,
            "a drained queue must reset the clock, or the next backfill's \
             first report waits out a window that started during this one"
        );
    }
}
