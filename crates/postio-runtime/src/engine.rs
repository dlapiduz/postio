//! The sync engine: one thread owns the connection and a current-thread runtime, state in
//! `Rc`/`RefCell`, work over a channel.
//! Callers await a reply and never block, because the caller is the UI.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use postio_account::auth::TokenSource;
use postio_account::backend::{BackendError, MailBackend};
use postio_core::event::MailFootprint;
use postio_model::RoleOverrides;
use postio_model::fetch::{PayloadAsk, Sections};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_smtp::transport::SmtpConnector;
use postio_storage::repository::{
    MailboxRepository, MessageRepository, OperationQueueRepository, SyncStateRepository,
};
use postio_storage::{BlobStore, Checkout, Store};
use postio_sync::backfill::Outcome;
use postio_sync::initial::Progress;
use postio_sync::status::StatusTracker;
use postio_sync::{
    AttachmentPolicy, Attention, Backfill, BackfillPolicy, BackfillProgress, Drainer, FilingPass,
    ReconnectPolicy, RetryPolicy, SmtpContext, Supervisor, SyncError, SyncStatus, Wake, Watch,
    WatchPolicy, Watcher, backfill, discover, initial, resync,
};

/// Re-exported from `postio-sync`; the frontend never sees these (this module is behind the
/// `runtime` feature).
pub use postio_sync::{Blocker, Link, NetworkState};

use postio_core::Event;
use postio_core::bridge::EventSink;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainSummary {
    pub applied: usize,
    pub coalesced: usize,
    pub obsolete: usize,
    pub deferred: usize,
    pub failed: Vec<String>,
    /// Separate from `failed`: a failure did not happen, this may have (#674).
    /// Nothing here becomes an `Event::Error`; the `Unconfirmed` draft asks the question.
    pub uncertain: Vec<String>,
    pub needs_resync: Vec<MailboxId>,
    pub thread_arrivals: postio_sync::threaded::ThreadArrivals,
    pub mailboxes_changed: bool,
}

impl DrainSummary {
    pub fn is_empty(&self) -> bool {
        self.applied == 0
            && self.coalesced == 0
            && self.obsolete == 0
            && self.deferred == 0
            && self.failed.is_empty()
            && self.uncertain.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub inserted: usize,
    pub updated: usize,
    pub threaded: usize,
    pub full: bool,
}

impl SyncSummary {
    pub fn changed(&self) -> bool {
        self.inserted > 0 || self.updated > 0 || self.threaded > 0
    }
}

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

pub struct EngineParts {
    pub account: AccountId,
    pub database: Store,
    pub blobs: BlobStore,
    pub backend: Arc<dyn MailBackend>,
    /// Absent means `Operation::Send` fails outright.
    pub smtp: Arc<dyn SmtpConnector>,
    /// Must be the same instance the IMAP store behind `backend` holds (ADR 0006 Q5): sharing lets
    /// a
    /// rejection on one side be seen on the other, and stops two refreshes of a rotating token
    /// invalidating each other.
    pub tokens: Arc<dyn TokenSource>,
    pub events: EventSink,
    pub retry: RetryPolicy,
    pub backfill: BackfillPolicy,
    pub reconnect: ReconnectPolicy,
    pub watch: WatchPolicy,
    pub network: NetworkSource,
    pub mailbox_roles: RoleOverrides,
    pub clock: Arc<dyn Clock>,
}

/// Explicit so a test never opens a D-Bus connection by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkSource {
    #[default]
    Ignored,
    NetworkManager,
}

enum Job {
    Drain {
        reply: tokio::sync::oneshot::Sender<Result<DrainSummary, EngineError>>,
    },
    RetryNow {
        reply: tokio::sync::oneshot::Sender<Link>,
    },
    SetNetwork {
        state: NetworkState,
        reply: tokio::sync::oneshot::Sender<Link>,
    },
    LinkState {
        reply: tokio::sync::oneshot::Sender<Link>,
    },
    BackfillProgress {
        reply: tokio::sync::oneshot::Sender<BackfillProgress>,
    },
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
    RequestPayloads {
        message: MessageId,
        parts: Vec<String>,
        reply: tokio::sync::oneshot::Sender<Result<PayloadAsk, EngineError>>,
    },
    RequestWholeMessage {
        message: MessageId,
        reply: tokio::sync::oneshot::Sender<Result<bool, EngineError>>,
    },
}

impl Job {
    /// A body request is an enqueue and a query reads engine state; neither needs a connection a
    /// lane holds or the write gate a suspended pass may hold (#1593).
    fn is_answered_beside_a_wave(&self) -> bool {
        matches!(
            self,
            Job::RequestBody { .. }
                | Job::RequestPayloads { .. }
                | Job::RequestWholeMessage { .. }
                | Job::SeedBackfill { .. }
                | Job::LinkState { .. }
                | Job::BackfillProgress { .. }
        )
    }

    fn asks_for_a_body(&self) -> bool {
        matches!(
            self,
            Job::RequestBody { .. } | Job::RequestPayloads { .. } | Job::RequestWholeMessage { .. }
        )
    }
}

/// Retained for the session so [`stop`] can join them: a sync pass may be mid-commit at exit, and
/// the
/// store's WAL recovery is not worth betting on.
static RETAINED: Mutex<Vec<Engine>> = Mutex::new(Vec::new());

pub fn retain(engine: Engine) {
    RETAINED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(engine);
}

/// Wind down a replaced engine without waiting: close its channel, drop it from the retained list,
/// and join off the caller's thread, which may be the UI loop (#1986).
pub fn retire(engine: Engine) {
    RETAINED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|kept| !Arc::ptr_eq(&kept.thread, &engine.thread));
    engine.jobs.close();
    JOB_SENT.notify_waiters();
    std::thread::spawn(move || engine.thread.join_bounded());
}

/// Stop `engines` and join their threads; other retained engines keep running (#1976).
pub fn stop(engines: Vec<Engine>) {
    RETAINED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|kept| {
            !engines
                .iter()
                .any(|engine| Arc::ptr_eq(&kept.thread, &engine.thread))
        });
    stop_and_wait(engines);
}

fn stop_and_wait(engines: Vec<Engine>) {
    if engines.is_empty() {
        return;
    }
    tracing::debug!(count = engines.len(), "stopping the sync engines");
    // Close every channel before joining, with one shared deadline: N accounts owe one grace, not N
    // (#759).
    for engine in &engines {
        engine.jobs.close();
    }
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    for engine in &engines {
        engine.thread.join_until(deadline);
    }
}

/// Cloning shares the thread; dropping the last handle waits for it to stop.
#[derive(Debug, Clone)]
pub struct Engine {
    /// Declared first so it drops first; reordering these fields reintroduces a deadlock.
    jobs: async_channel::Sender<Job>,
    /// Never read; the last handle to go joins the thread in `Drop`.
    #[allow(dead_code)]
    thread: Arc<EngineThread>,
    filing: FilingSlot,
}

/// Where a Focus-mode host puts the filing pass: one slot shared by every engine it starts, read
/// once as each pass starts.
#[derive(Clone, Default)]
pub struct FilingSlot(Arc<std::sync::RwLock<Option<Arc<dyn FilingPass>>>>);

impl FilingSlot {
    pub fn get(&self) -> Option<Arc<dyn FilingPass>> {
        self.0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set(&self, pass: Option<Arc<dyn FilingPass>>) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = pass;
    }
}

impl fmt::Debug for FilingSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilingSlot")
            .field("filing", &self.get().is_some())
            .finish()
    }
}

/// Joined when the last handle goes: `exit()` does not stop threads, and a pass mid-commit must not
/// be torn.
/// The wait is bounded by `SHUTDOWN_GRACE` because `Drop` often runs on the GTK main loop; giving
/// up detaches.
#[derive(Debug)]
struct EngineThread {
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    busy: Busy,
}

#[derive(Clone, Debug)]
struct Busy(Arc<Mutex<String>>);

impl Busy {
    fn new() -> Self {
        Self(Arc::new(Mutex::new("starting up".to_owned())))
    }

    fn set(&self, what: impl Into<String>) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = what.into();
    }

    fn what(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// Normally microseconds; the bound covers a job stuck in a network read.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

impl EngineThread {
    /// Idempotent. The caller must close the job channel first, or this just spends the grace.
    fn join_bounded(&self) {
        self.join_until(Instant::now() + SHUTDOWN_GRACE);
    }

    /// Against a caller-chosen deadline, so N engines share one grace (#759).
    fn join_until(&self, deadline: Instant) {
        let Some(handle) = self
            .handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };

        // The engine dropping its own handle: joining would wait on itself.
        if handle.thread().id() == std::thread::current().id() {
            return;
        }

        while !handle.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            // Detached deliberately: a stalled engine must not hold the window shut.
            tracing::warn!(
                busy = %self.busy.what(),
                "the sync engine did not stop within {SHUTDOWN_GRACE:?}; leaving it running"
            );
        }
    }
}

impl Drop for EngineThread {
    fn drop(&mut self) {
        self.join_bounded();
    }
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
            Job::RequestPayloads { message, parts, .. } => {
                write!(
                    formatter,
                    "RequestPayloads({message}, {} parts)",
                    parts.len()
                )
            }
            Job::RequestWholeMessage { message, .. } => {
                write!(formatter, "RequestWholeMessage({message})")
            }
        }
    }
}

/// A bare `std::thread` gets 2 MiB, not the main thread's 8 MiB, and `block_on` of the sync loop
/// stacks
/// deep frames: the smallest possible sync peaked near 1 MiB and real ones aborted (#1541).
const ENGINE_STACK: usize = 8 * 1024 * 1024;

impl Engine {
    pub fn spawn(parts: EngineParts) -> Result<Engine, EngineError> {
        Engine::spawn_filing(parts, FilingSlot::default())
    }

    pub fn spawn_filing(parts: EngineParts, filing: FilingSlot) -> Result<Engine, EngineError> {
        // Unbounded: the sender is the UI and must never block on the engine.
        let (jobs, inbox) = async_channel::unbounded::<Job>();
        let store = parts.database.clone();
        let busy = Busy::new();

        let handle = std::thread::Builder::new()
            .name("postio-sync".to_string())
            .stack_size(ENGINE_STACK)
            .spawn({
                let busy = busy.clone();
                let filing = filing.clone();
                move || run(parts, store, inbox, busy, filing)
            })
            .map_err(|error| EngineError::new(format!("the sync engine did not start: {error}")))?;

        Ok(Engine {
            jobs,
            thread: Arc::new(EngineThread {
                handle: Mutex::new(Some(handle)),
                busy,
            }),
            filing,
        })
    }

    pub fn files_arrivals(&self) -> bool {
        self.filing.get().is_some()
    }

    pub fn is_stopped(&self) -> bool {
        self.jobs.is_closed()
    }

    /// Stop and join the thread whatever else holds a handle; the retained handles never drop.
    /// Idempotent; not from the engine's own thread.
    pub fn stop(&self) {
        self.jobs.close();
        JOB_SENT.notify_waiters();
        self.thread.join_bounded();
    }

    pub async fn drain(&self) -> Result<DrainSummary, EngineError> {
        self.ask(|reply| Job::Drain { reply }).await
    }

    /// The one thing that clears `Link::Blocked`: a refused password does not improve on a timer.
    pub async fn retry_now(&self) -> Result<Link, EngineError> {
        self.tell(|reply| Job::RetryNow { reply }).await
    }

    /// `Link::Offline` is not `Link::Waiting`: with no network no attempts are spent.
    pub async fn set_network(&self, state: NetworkState) -> Result<Link, EngineError> {
        self.tell(|reply| Job::SetNetwork { state, reply }).await
    }

    pub async fn link(&self) -> Result<Link, EngineError> {
        self.tell(|reply| Job::LinkState { reply }).await
    }

    /// Every queued message is in exactly one count, so a progress display adds up.
    pub async fn backfill_progress(&self) -> Result<BackfillProgress, EngineError> {
        self.tell(|reply| Job::BackfillProgress { reply }).await
    }

    pub async fn sync(&self, mailbox: MailboxId) -> Result<SyncSummary, EngineError> {
        self.ask(|reply| Job::Sync { mailbox, reply }).await
    }

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

    /// `false` means nothing to fetch: the body is here or the message is gone.
    pub async fn request_body(&self, message: MessageId) -> Result<bool, EngineError> {
        self.ask(|reply| Job::RequestBody { message, reply }).await
    }

    /// Says whether the fetch was queued and, if not, why (#1996); `Unreachable` is queued all the
    /// same
    /// for when the link returns.
    pub async fn request_payloads(
        &self,
        message: MessageId,
        parts: Vec<String>,
    ) -> Result<PayloadAsk, EngineError> {
        self.ask(|reply| Job::RequestPayloads {
            message,
            parts,
            reply,
        })
        .await
    }

    /// Every byte, not only the text sections; on demand only (ADR 0017).
    pub async fn request_whole_message(&self, message: MessageId) -> Result<bool, EngineError> {
        self.ask(|reply| Job::RequestWholeMessage { message, reply })
            .await
    }

    async fn tell<T>(
        &self,
        job: impl FnOnce(tokio::sync::oneshot::Sender<T>) -> Job,
    ) -> Result<T, EngineError> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.jobs
            .send(job(reply))
            .await
            .map_err(|_| EngineError::new("the sync engine has stopped"))?;
        JOB_SENT.notify_waiters();
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
        JOB_SENT.notify_waiters();
        answer
            .await
            .unwrap_or_else(|_| Err(EngineError::new("the sync engine dropped the work")))
    }
}

/// Bodies fetched between two sync waves, so a mailbox is readable while the rest is enumerated:
/// zero starves bodies, unbounded starves the headers of a huge folder.
const BODIES_BETWEEN_WAVES: usize = 8;

/// Bodies of one batch written concurrently beside the lanes. Draining the whole queue in the
/// pass-completion handler froze every lane, and a handler must not take the write gate.
const BODIES_BESIDE_A_WAVE: usize = 8;

/// One batch writing while the next is on the wire; a batch not yet started when a job arrives goes
/// back (#1941).
const BATCHES_BESIDE_A_WAVE: usize = 2;

/// **Without this a long download makes the app deaf**: the loop drains the body queue before it
/// watches for arrivals, which on a large first sync is hours. Expiring only asks the watcher
/// whether
/// anything is due, so this is a delivery's latency, not a round-trip rate.
const BACKFILL_SLICE: Duration = POLL_INTERVAL;

/// Public: Focus mode's due timer keeps the same tick (spec 007).
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// One worker, but multi-threaded: a `current_thread` runtime refuses `block_in_place`, which a
/// synchronous store read needs (`postio_session::blocking`).
fn run(
    parts: EngineParts,
    store: Store,
    inbox: async_channel::Receiver<Job>,
    busy: Busy,
    filing: FilingSlot,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        // Same as `ENGINE_STACK`: a spawned task lands on this worker, which also gets the 2 MiB
        // default.
        .thread_stack_size(ENGINE_STACK)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "the sync engine has no runtime: {error}");
            parts.events.emit(Event::Error {
                message: format!("the sync engine has no runtime: {error}"),
                account: None,
            });
            return;
        }
    };

    let span = tracing::info_span!("engine", account = parts.account.get());
    runtime.block_on(tracing::Instrument::instrument(
        async move {
            let mut state = State {
                busy,
                backfill: Backfill::new(parts.backfill),
                backfill_announced: None,
                footprint: None,
                supervisor: Supervisor::new(parts.reconnect),
                status: Rc::new(RefCell::new(StatusTracker::new())),
                online: false,
                to_sync: std::collections::VecDeque::new(),
                inbox_ids: Vec::new(),
                inbox_page_pending: false,
                failing: Default::default(),
                watcher: None,
                backfill_covered: false,
                filing,
            };
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            // A channel, not a job, so a slow or absent bus cannot hold up anything the user asked
            // for.
            let (network, mut network_changed) = tokio::sync::watch::channel(NetworkState::Unknown);
            if parts.network == NetworkSource::NetworkManager {
                tokio::spawn(crate::network::follow(network));
            }

            // Connect now rather than race `ticker`'s first tick: with a job already queued,
            // `select!`
            // picks among ready branches arbitrarily, and the race cost a full `POLL_INTERVAL`
            // (#109).
            let moved = state
                .supervisor
                .poll(parts.backend.as_ref(), Utc::now(), entropy())
                .await;
            announce_link(&parts, &mut state, moved);
            handle_link_transition(&parts, &store, &mut state).await;

            loop {
                // Computed before `select!` so the branch holds a number, not a borrow of `state`.
                let link_due = match state.supervisor.link() {
                    postio_sync::Link::Waiting { retry_at, .. } => {
                        Some((*retry_at - Utc::now()).to_std().unwrap_or(Duration::ZERO))
                    }
                    _ => None,
                };
                tokio::select! {
                    job = inbox.recv() => match job {
                        Ok(job) => {
                            state.busy.set(format!("serving {job:?}"));
                            serve(job, &parts, &store, &mut state).await;
                        }
                        Err(_) => break,
                    },
                    // The link's own retry time, not the next tick: a body asked for during a short
                    // backoff was told the link was down (#1985).
                    () = async {
                        match link_due {
                            Some(wait) => tokio::time::sleep(wait).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let moved = state
                            .supervisor
                            .poll(parts.backend.as_ref(), Utc::now(), entropy())
                            .await;
                        announce_link(&parts, &mut state, moved);
                    }
                    _ = ticker.tick() => {
                        let moved = state
                            .supervisor
                            .poll(parts.backend.as_ref(), Utc::now(), entropy())
                            .await;
                        announce_link(&parts, &mut state, moved);
                        wake_due_snoozes(&parts, &store).await;
                    }
                    // Queued work wakes the drain without waiting for `ticker` (4.0 s measured for
                    // a flag on an idle app).
                    () = wait_for_queued_work(&parts, &store) => {}
                    // Only moves the link between waiting and offline; the attempt count is the
                    // supervisor's, since NetworkManager knows interfaces, not servers.
                    Ok(()) = network_changed.changed() => {
                        let reported = *network_changed.borrow_and_update();
                        let moved = state.supervisor.set_network(reported, Utc::now());
                        announce_link(&parts, &mut state, moved);
                    }
                }

                // Act on the transition first so a queue waiting for a connection goes out the
                // moment there is one.
                handle_link_transition(&parts, &store, &mut state).await;

                let slice_ends = Instant::now() + BACKFILL_SLICE;

                // A few mailboxes at a time with the inbox checked between waves; a job cancels a
                // wave in progress (see `sync_wave`).
                while nothing_asked(&inbox)
                    && state.supervisor.link().is_online()
                    && !state.to_sync.is_empty()
                    && !has_queued_work(&parts, &store).await
                {
                    state.busy.set("a sync wave");
                    if let Some(job) = sync_wave(&parts, &store, &mut state, &inbox).await {
                        // The wave took this off the inbox and could not answer it beside its
                        // lanes; serve it first.
                        state.busy.set(format!("serving {job:?}"));
                        serve(job, &parts, &store, &mut state).await;
                    }

                    // Bodies between waves: a sync seeds the backfill for the folder it changed,
                    // and without this
                    // pump the inbox's bodies waited until every folder was enumerated.
                    // Every guard of the loop below applies: a job, queued operation or going
                    // offline outranks a body nobody asked for.
                    let mut fetched = 0;
                    while fetched < BODIES_BETWEEN_WAVES
                        && Instant::now() < slice_ends
                        && nothing_asked(&inbox)
                        && state.supervisor.link().is_online()
                        && !has_queued_work(&parts, &store).await
                    {
                        // Refill before pumping: pumping alone drains the last seed and stops;
                        // `top_up_backfill` finds the next batch.
                        if state.backfill.is_idle()
                            && top_up_backfill(&parts, &store, &mut state).await == 0
                        {
                            break;
                        }
                        if !pump_body(&parts, &store, &mut state, &inbox, slice_ends, 1).await {
                            break;
                        }
                        fetched += 1;
                    }
                }

                // A wave that yielded to queued work must be followed by the drain it yielded for,
                // or the operation waits up to `POLL_INTERVAL` (#944).
                if state.supervisor.link().is_online() && has_queued_work(&parts, &store).await {
                    state.busy.set("draining after a wave");
                    let outcome = drain(&parts, &store, &mut state).await;
                    announce_drain(&parts, &outcome).await;
                }

                // Then bodies, only while nothing else is asking: a body nobody asked for is a
                // guess and must not outrank a queued operation.
                while Instant::now() < slice_ends
                    && nothing_asked(&inbox)
                    && state.supervisor.link().is_online()
                    && !state.backfill.is_idle()
                    && !has_queued_work(&parts, &store).await
                    && pump_body(&parts, &store, &mut state, &inbox, slice_ends, usize::MAX).await
                {
                }

                // Then queue the next batch (#318), only once the queue has drained: `is_idle` is
                // false while the
                // lane is merely paused, and topping up then would pull in an archive that policy
                // said not to fetch.
                while Instant::now() < slice_ends
                    && nothing_asked(&inbox)
                    && state.supervisor.link().is_online()
                    && state.backfill.is_idle()
                    && !state.backfill_covered
                    && !has_queued_work(&parts, &store).await
                    && top_up_backfill(&parts, &store, &mut state).await > 0
                {
                    while Instant::now() < slice_ends
                        && nothing_asked(&inbox)
                        && state.supervisor.link().is_online()
                        && !state.backfill.is_idle()
                        && !has_queued_work(&parts, &store).await
                        && pump_body(&parts, &store, &mut state, &inbox, slice_ends, usize::MAX)
                            .await
                    {}
                }

                // Then wait for mail, last because it idles. `keep_watch` is never dropped
                // mid-step: a dropped
                // step leaves its mailbox unwatched and the store a session still inside `IDLE`
                // (#755).
                if inbox.is_empty() && state.to_sync.is_empty() {
                    // A full slice means a download is in progress; a held `IDLE` would starve it,
                    // so the watch takes the cheap step.
                    let downloading = Instant::now() >= slice_ends;
                    if !downloading {
                        state.busy.set("idle");
                    }
                    keep_watch(&parts, &store, &mut state, &inbox, downloading).await;
                    // Defence in depth: a step that vanished unreported must not leave its mailbox
                    // treated as covered.
                    if let Some(watcher) = state.watcher.as_mut() {
                        watcher.interrupted(Utc::now());
                    }
                }
            }
        },
        span,
    ));
}

struct State {
    busy: Busy,
    backfill: Backfill,
    backfill_announced: Option<std::time::Instant>,
    /// Cached so a status-line redraw does not cost a table scan.
    footprint: Option<MailFootprint>,
    supervisor: Supervisor,
    /// A `RefCell` because a wave's passes fold batches in concurrently; no borrow is held across
    /// an await.
    /// `Rc` so `sync_wave` can clone a handle each wave (#631).
    status: Rc<RefCell<StatusTracker>>,
    online: bool,
    to_sync: std::collections::VecDeque<MailboxId>,
    /// Keyed on the role, never a folder name.
    inbox_ids: Vec<MailboxId>,
    /// INBOX's headers are done and its first page of bodies is seeded but not fetched; no other
    /// mailbox is admitted until it clears.
    inbox_page_pending: bool,
    /// Mailboxes whose last pass failed with the link up (#1731), so the next completed pass clears
    /// the mark once.
    failing: std::collections::BTreeSet<MailboxId>,
    /// `None` until the first connection reports whether the server offers `IDLE`.
    watcher: Option<Watcher>,
    /// A latch stopping a per-folder query every loop once covered; cleared by a sync that wrote
    /// and by the link coming up.
    backfill_covered: bool,
    filing: FilingSlot,
}

/// For tests: waits that woke on their own clock rather than on a send or a write.
static IDLE_WAKEUPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[doc(hidden)]
pub fn idle_wakeups() -> u64 {
    IDLE_WAKEUPS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Silent about failure: a connection this cannot check out is reported elsewhere, and a skipped
/// drain costs one tick.
async fn has_queued_work(parts: &EngineParts, store: &Store) -> bool {
    let Ok(connection) = store.connect_background().await else {
        return false;
    };
    OperationQueueRepository::new(&connection)
        .pending(parts.account, Utc::now())
        .await
        .is_ok_and(|due| !due.is_empty())
}

/// Visibility is self-healing (list queries read `snoozed_until` fresh); this only makes a due row
/// appear
/// within one `POLL_INTERVAL` (#493). Silent about failure, like `has_queued_work`.
async fn wake_due_snoozes(parts: &EngineParts, store: &Store) {
    let Ok(connection) = store.connect_background().await else {
        return;
    };
    let Ok(woken) = MessageRepository::new(&connection)
        .wake_due(parts.account, Utc::now())
        .await
    else {
        return;
    };
    for mailbox in woken {
        parts.events.emit(Event::MessageListChanged {
            account: parts.account,
            mailbox,
        });
    }
}

fn came_up(state: &mut State) -> bool {
    let online = state.supervisor.link().is_online();
    let transition = online && !state.online;
    state.online = online;
    transition
}

/// Shared with the one connection attempt before the loop, so a link up before the first iteration
/// is handled like one mid-loop.
async fn handle_link_transition(parts: &EngineParts, store: &Store, state: &mut State) {
    if came_up(state) {
        // A link just up is when bodies set aside while it was down are worth offering again.
        state.backfill_covered = false;
        state.backfill.reconnected();
        let outcome = drain(parts, store, state).await;
        announce_drain(parts, &outcome).await;
        // Folders first: everything below reads the local table, which is empty on a new account.
        discover(parts, store).await;
        queue_every_mailbox(parts, store, state).await;
        start_watching(parts, store, state).await;
    } else if state.supervisor.link().is_online() && has_queued_work(parts, store).await {
        // Writers cannot tell this thread they wrote a row, so it asks (one indexed read);
        // otherwise a
        // mutation made while connected would wait for the next reconnection.
        let outcome = drain(parts, store, state).await;
        announce_drain(parts, &outcome).await;
    }
}

/// Observes only: `recv` would take the job and lose it when the other `select!` branch cancels it.
async fn wait_for_job(inbox: &async_channel::Receiver<Job>) {
    loop {
        let sent = JOB_SENT.notified();
        tokio::pin!(sent);
        sent.as_mut().enable();
        if !nothing_asked(inbox) {
            return;
        }
        // Woken by a send; the tick is for the inbox closing, which no send announces.
        tokio::select! {
            () = sent => {}
            () = tokio::time::sleep(CLOSE_CHECK) => {
                IDLE_WAKEUPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

/// Process-wide because every wait rechecks its own inbox; a wake meant for another engine costs
/// one `is_empty`.
static JOB_SENT: tokio::sync::Notify = tokio::sync::Notify::const_new();

const CLOSE_CHECK: Duration = Duration::from_secs(1);

/// A closed channel is also empty, so `is_empty` alone reads shutdown as leisure (#759).
fn nothing_asked(inbox: &async_channel::Receiver<Job>) -> bool {
    inbox.is_empty() && !inbox.is_closed()
}

async fn wait_for_close(inbox: &async_channel::Receiver<Job>) {
    while !inbox.is_closed() {
        tokio::time::sleep(CLOSE_POLL).await;
    }
}

/// Tighter than `CLOSE_CHECK`: at a second this was half the shutdown budget (#1941).
const CLOSE_POLL: Duration = Duration::from_millis(50);

/// Polled because the queue is a table written by other threads and gives no signal; two indexed
/// reads
/// a second get a connected action to the server promptly.
async fn wait_for_queued_work(parts: &EngineParts, store: &Store) {
    loop {
        // Enabled before the check, so a write between the check and the wait still wakes it.
        let written = store.write_gate().interactive_writes().notified();
        tokio::pin!(written);
        written.as_mut().enable();
        if has_queued_work(parts, store).await {
            return;
        }
        // Woken by interactive writes; the tick covers what no write announces, an operation whose
        // retry comes due.
        tokio::select! {
            () = written => {}
            () = tokio::time::sleep(POLL_INTERVAL) => {
                IDLE_WAKEUPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

const WATCH_FLOOR: Duration = Duration::from_millis(50);

/// The inbox gets the dedicated connection; everything else is checked on an interval over the
/// shared one.
#[tracing::instrument(skip_all)]
async fn start_watching(parts: &EngineParts, store: &Store, state: &mut State) {
    let capabilities = match parts.backend.capabilities().await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            tracing::warn!(%error, "cannot read the server's capabilities; not watching");
            return;
        }
    };
    // Names only: a server's capability list is public, not the user's data.
    tracing::debug!(
        capabilities = ?capabilities.names(),
        incremental = capabilities.supports_incremental_sync(),
        "server capabilities"
    );
    let mut watcher = Watcher::new(parts.watch, &capabilities);

    let Ok(connection) = store.connect_background().await else {
        tracing::warn!("no connection to read folders with; not watching");
        return;
    };
    let mailboxes = match MailboxRepository::new(&connection)
        .list_for_account(parts.account)
        .await
    {
        Ok(mailboxes) => mailboxes,
        Err(error) => {
            tracing::error!(%error, "cannot read the account's folders; not watching: {error}");
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

/// Lets `keep_watch` end a held `IDLE` early without being dropped mid-command; only observes.
async fn interruption(parts: &EngineParts, store: &Store, inbox: &async_channel::Receiver<Job>) {
    tokio::select! {
        _ = wait_for_job(inbox) => {}
        // A local write is not a job, and an `IDLE` is held for minutes.
        _ = wait_for_queued_work(parts, store) => {}
    }
}

enum Asked {
    Nothing,
    FetchNow,
    DrainNow,
    Stop(Option<Job>),
}

/// A wave used to cancel every lane for any job or due queue, restarting first-sync passes once per
/// message
/// the person opened (#1593). Now a body request is enqueued, a query answered, and a due operation
/// drained
/// beside the lanes; only an unanswerable job stops the wave, and it is handed back because a
/// channel has no front.
async fn attend(
    parts: &EngineParts,
    store: &Store,
    state: &mut State,
    inbox: &async_channel::Receiver<Job>,
    draining: bool,
) -> Asked {
    match inbox.try_recv() {
        Ok(job) if job.is_answered_beside_a_wave() => {
            let wants_a_body = job.asks_for_a_body();
            serve(job, parts, store, state).await;
            if wants_a_body {
                Asked::FetchNow
            } else {
                Asked::Nothing
            }
        }
        Ok(job) => Asked::Stop(Some(job)),
        Err(async_channel::TryRecvError::Closed) => Asked::Stop(None),
        Err(async_channel::TryRecvError::Empty) => {
            if !draining && has_queued_work(parts, store).await {
                Asked::DrainNow
            } else {
                Asked::Nothing
            }
        }
    }
}

/// Interruption is cooperative: the step's cancel token is fired and the command awaited, never
/// dropped;
/// a dropped command leaves the store a session still inside `IDLE` (#755).
async fn keep_watch(
    parts: &EngineParts,
    store: &Store,
    state: &mut State,
    inbox: &async_channel::Receiver<Job>,
    downloading: bool,
) {
    if !state.supervisor.link().is_online() {
        // Nothing to watch over: wait. Logged at debug because a `Blocked` link never retries on
        // its
        // own, and silence looks like a hang.
        tracing::debug!(
            link = %postio_model::address::redact_addresses(&format!("{:?}", state.supervisor.link())),
            "not connected; nothing to watch"
        );
        // Not past a backoff's due time, or a body asked for meanwhile is told the link is down
        // (#1985).
        let wait = match state.supervisor.link() {
            postio_sync::Link::Waiting { retry_at, .. } => (*retry_at - Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO)
                .min(POLL_INTERVAL),
            _ => POLL_INTERVAL,
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = interruption(parts, store, inbox) => {}
        }
        return;
    }
    if state.watcher.is_none() {
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = interruption(parts, store, inbox) => {}
        }
        return;
    }
    let Some(watcher) = state.watcher.as_mut() else {
        return;
    };

    let now = Utc::now();
    // Push first, unless a download is in progress: an `IDLE` is held for minutes and the backfill
    // does
    // not wake `interruption`, so a one-round-trip `STATUS` is used instead.
    let step = match watcher.next_push(now) {
        Watch::Wait { .. } => watcher.next_poll(now),
        step => step,
    };
    // During a download keep the same mailbox but change the step's shape: `next_poll` never offers
    // the
    // pushed mailbox (the inbox), so asking it would stop the inbox being watched at all.
    let step = match (downloading, step) {
        (true, Watch::Idle { mailbox, path, .. }) => Watch::Poll { mailbox, path },
        (true, Watch::Wait { .. }) => return,
        (_, step) => step,
    };

    let watched = step_mailbox(&step);
    let wake = match step {
        Watch::Idle {
            mailbox,
            path,
            timeout,
            cancel,
        } => {
            // Run to completion: an interruption fires the token and waits; dropping returns a
            // session
            // still inside `IDLE` and the next checkout hangs (#755).
            let command = parts.backend.idle(&path, timeout, &cancel);
            tokio::pin!(command);
            let result = tokio::select! {
                biased;
                result = &mut command => result,
                () = interruption(parts, store, inbox) => {
                    cancel.cancel();
                    command.await
                }
            };
            match result {
                Ok(events) => watcher.woke(mailbox, &events, Utc::now()),
                Err(error) => {
                    watcher.failed(mailbox, Utc::now());
                    let moved = state.supervisor.observe(&error, Utc::now());
                    announce_link(parts, state, moved);
                    return;
                }
            }
        }
        // An interruption waits out a `STATUS` rather than dropping a half-read reply into the
        // store.
        Watch::Poll { mailbox, path } => match parts.backend.status(&path).await {
            Ok(status) => watcher.observed(mailbox, &status, Utc::now()),
            Err(error) => {
                watcher.failed(mailbox, Utc::now());
                let moved = state.supervisor.observe(&error, Utc::now());
                announce_link(parts, state, moved);
                return;
            }
        },
        Watch::Wait { until } => {
            let delay = until
                .map(|at| (at - Utc::now()).to_std().unwrap_or_default())
                .unwrap_or(POLL_INTERVAL);
            tokio::select! {
                _ = tokio::time::sleep(delay.clamp(WATCH_FLOOR, POLL_INTERVAL)) => {}
                _ = interruption(parts, store, inbox) => {}
            }
            return;
        }
    };

    if let (Wake::Changed, Some(mailbox)) = (wake, watched) {
        // The events say only that the mailbox moved, not what, so pull.
        if !state.to_sync.contains(&mailbox) {
            state.to_sync.push_back(mailbox);
        }
    }
}

fn step_mailbox(step: &Watch) -> Option<MailboxId> {
    match step {
        Watch::Idle { mailbox, .. } | Watch::Poll { mailbox, .. } => Some(*mailbox),
        _ => None,
    }
}

/// Runs on every reconnection: folders change under other clients, and a new account's local table
/// is empty.
/// A failure is reported and not fatal.
async fn discover(parts: &EngineParts, store: &Store) {
    let connection = match store.connect_background().await {
        Ok(connection) => connection,
        Err(error) => {
            parts.events.emit(Event::Error {
                message: error.to_string(),
                account: Some(parts.account),
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
                parts.events.emit(Event::MailboxesChanged {
                    account: parts.account,
                });
            }
        }
        Err(error) => {
            parts.events.emit(Event::Error {
                message: error.to_string(),
                account: Some(parts.account),
            });
        }
    }
}

/// Each call asks for the next batch because a landed body leaves `needing_backfill`, so the walk
/// lives in
/// durable rows (#318). Candidates over `max_body_bytes` or failed this session are set aside and
/// count
/// zero, so the caller stops; `backfill_covered` stops the re-asking.
async fn top_up_backfill(parts: &EngineParts, store: &Store, state: &mut State) -> usize {
    if state.backfill_covered {
        return 0;
    }
    let Ok(connection) = store.connect_background().await else {
        // Not latched: the next pass will have a connection.
        return 0;
    };
    let mut mailboxes = match MailboxRepository::new(&connection)
        .list_for_account(parts.account)
        .await
    {
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
        )
        .await
        {
            Ok(queued) => queued,
            Err(error) => {
                tracing::warn!(%error, "cannot top up the backfill for a folder");
                continue;
            }
        };
        if queued > 0 {
            // Counts and a folder id, all a log may carry about mail.
            tracing::debug!(mailbox = mailbox.id.get(), queued, "backfill topped up");
            announce_backfill(parts, state, std::time::Instant::now()).await;
            return queued;
        }
    }

    // Header blocks for rows that cannot rebuild one from disk (#884); unconditional because it is
    // a
    // migration finishing, not a policy.
    for mailbox in mailboxes.iter().filter(|mailbox| mailbox.selectable) {
        let queued = match backfill::seed_header_blocks(
            &connection,
            &mut state.backfill,
            mailbox.id,
            parts.backfill.seed_batch,
        )
        .await
        {
            Ok(queued) => queued,
            Err(error) => {
                tracing::warn!(%error, "cannot top up the header blocks for a folder");
                continue;
            }
        };
        if queued > 0 {
            tracing::debug!(
                mailbox = mailbox.id.get(),
                queued,
                "header blocks topped up"
            );
            announce_backfill(parts, state, std::time::Instant::now()).await;
            return queued;
        }
    }

    // Payloads only after every folder's words and only if asked (ADR 0017): text is what makes
    // search
    // complete, and attachments first would leave a mailbox unsearchable.
    if parts.backfill.attachments == AttachmentPolicy::Eager {
        for mailbox in mailboxes.iter().filter(|mailbox| mailbox.selectable) {
            let queued = match backfill::seed_payloads(
                &connection,
                &mut state.backfill,
                mailbox.id,
                parts.backfill.seed_batch,
            )
            .await
            {
                Ok(queued) => queued,
                Err(error) => {
                    tracing::warn!(%error, "cannot top up the payloads for a folder");
                    continue;
                }
            };
            if queued > 0 {
                tracing::debug!(mailbox = mailbox.id.get(), queued, "payloads topped up");
                announce_backfill(parts, state, std::time::Instant::now()).await;
                return queued;
            }
        }
    }

    state.backfill_covered = true;
    tracing::debug!("every folder's bodies are local, or set aside");
    0
}

/// INBOX first by `sync_priority`: a large Archive queued ahead left an empty inbox for as long as
/// it took.
async fn queue_every_mailbox(parts: &EngineParts, store: &Store, state: &mut State) {
    let Ok(connection) = store.connect_background().await else {
        tracing::warn!("no connection to read folders with; syncing nothing this pass");
        return;
    };
    let mut mailboxes = match MailboxRepository::new(&connection)
        .list_for_account(parts.account)
        .await
    {
        Ok(mailboxes) => mailboxes,
        Err(error) => {
            tracing::error!(%error, "cannot read the account's folders; syncing nothing: {error}");
            return;
        }
    };
    mailboxes.sort_by_key(|mailbox| postio_sync::order::sync_priority(mailbox.role));
    state.inbox_ids = mailboxes
        .iter()
        .filter(|mailbox| mailbox.role == postio_model::MailboxRole::Inbox)
        .map(|mailbox| mailbox.id)
        .collect();
    state.to_sync.clear();
    state.to_sync.extend(
        mailboxes
            .iter()
            .filter(|mailbox| mailbox.selectable)
            .map(|mailbox| mailbox.id),
    );
    // A failed folder that is no longer one to sync will never have the pass that clears its mark.
    let gone: Vec<MailboxId> = state
        .failing
        .iter()
        .filter(|mailbox| !state.to_sync.contains(mailbox))
        .copied()
        .collect();
    for mailbox in gone {
        state.failing.remove(&mailbox);
        parts.events.emit(Event::MailboxSyncChanged {
            account: parts.account,
            mailbox,
            failure: None,
        });
    }
    // Answers "connected and nothing happened": zero here means `discover` has not filled the local
    // table or found nothing.
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

/// Each notification costs the list an invalidation and a page read; one per batch would be
/// hundreds of reloads on a first sync.
const REPAINT_INTERVAL: Duration = Duration::from_millis(500);

/// Injected so a test can make the event count a function of the batches synced, not of wall-clock
/// time (#507).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The first batch is announced immediately and later ones throttled: the first screenful is what
/// the user waits for.
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

    /// Only a full enumeration gets here. The event discards every visible row widget, which is
    /// wrong for
    /// an incremental delivery, so an incremental path must not come through here.
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

/// Both halves happen as the batch lands; folding them in after the pass left the list empty and
/// the
/// status silent. The repaint is held to `REPAINT_INTERVAL`; `StatusTracker` has its own interval
/// and
/// never drops the report that completes a pass.
struct Committed<'a> {
    parts: &'a EngineParts,
    /// A cell because a wave's passes share the tracker; the borrow is taken and returned inside
    /// `batch`, never across an await.
    status: &'a RefCell<StatusTracker>,
    repaint: Repaint,
    yield_point: &'a tokio::sync::Notify,
}

impl Committed<'_> {
    fn batch(&mut self, progress: Progress) {
        self.repaint.batch_committed(self.parts.clock.now());
        self.yield_point.notify_one();
        let reported = self.status.borrow_mut().on_progress(progress, Utc::now());
        if let Some(status) = reported {
            // Counts and an outcome, all a log may carry about mail.
            tracing::debug!(
                done = progress.fetched,
                total = progress.target,
                "sync progress announced"
            );
            announce_status(self.parts, &status);
        }
    }
}

/// Fetch queued bodies a batch at a time, up to `BODY_WINDOW` writes at once; returns whether it
/// did anything.
/// A priority picks who takes the next free connection, not who loses one, so the general lane must
/// never be
/// full of background fetches; the window leaves one general connection for the reader (#1551).
async fn pump_body(
    parts: &EngineParts,
    store: &Store,
    state: &mut State,
    inbox: &async_channel::Receiver<Job>,
    until: Instant,
    batches: usize,
) -> bool {
    let first = state.backfill.next_batch();
    let Some(head) = first.first() else {
        return false;
    };
    // Every claim carries the backfill's one token, so this cancels all of them (shutdown).
    let cancel = head.cancel.clone();

    // Read before the fetches start so nothing borrows `state` while they are in flight (ADR 0017,
    // #751).
    let inline_cap = state.backfill.policy().max_inline_bytes;

    // One batch is written while the next is on the wire (#1941); the look-ahead is claimed only
    // while
    // every guard still holds, and a claim for a batch that does not start goes back to its queue.
    let mut ready = prefetch_watched(parts, store, inbox, first).await;
    let mut done = 0;
    loop {
        done += 1;
        let (claims, prefetched) = ready;
        let ahead_claims = if done < batches
            && Instant::now() < until
            && nothing_asked(inbox)
            && state.supervisor.link().is_online()
            && !has_queued_work(parts, store).await
        {
            state.backfill.next_batch()
        } else {
            Vec::new()
        };
        let mut ahead = (!ahead_claims.is_empty())
            .then(|| Box::pin(prefetch_watched(parts, store, inbox, ahead_claims)));
        let mut arrived = None;

        state.busy.set(match claims.as_slice() {
            [only] => format!("the body backfill (message {})", only.request.message),
            many => format!("the body backfill ({} messages)", many.len()),
        });
        let mut waiting: std::collections::VecDeque<_> = claims.into();
        let mut running = FuturesUnordered::new();

        // Shutdown must not wait for the wire: fire the token when the channel closes, since the
        // command
        // timeout is twelve times the grace (#759). In-flight fetches are still drained so no claim
        // is left outstanding.
        let mut cancelled = false;
        loop {
            // A job arriving mid-batch waits at most for the bodies on the wire; what has not
            // started goes back.
            if !nothing_asked(inbox) {
                for claim in waiting.drain(..) {
                    state.backfill.release(claim);
                }
            }
            while running.len() < BODY_WINDOW {
                let Some(claim) = waiting.pop_front() else {
                    break;
                };
                running.push(fetch_one_body(
                    parts,
                    store,
                    inline_cap,
                    prefetched.clone(),
                    claim,
                ));
            }
            if running.is_empty() {
                break;
            }
            let settled = tokio::select! {
                biased;
                next = running.next() => next,
                landed = async { ahead.as_mut().expect("guarded").await },
                    if ahead.is_some() => {
                    arrived = Some(landed);
                    ahead = None;
                    continue;
                }
                () = wait_for_close(inbox), if !cancelled => {
                    cancel.cancel();
                    cancelled = true;
                    continue;
                }
            };
            let Some(Settled {
                message,
                mailbox,
                sections,
                result,
            }) = settled
            else {
                break;
            };
            // A slot just freed: start a just-opened message now rather than behind the window;
            // interactive only,
            // since a background top-up would make the call unbounded.
            if !cancelled && let Some(next) = state.backfill.next_interactive_body() {
                running.push(fetch_one_body(parts, store, inline_cap, None, next));
            }
            settle_body(parts, state, message, mailbox, sections, result).await;
        }

        let next = match (arrived, ahead) {
            (Some(landed), _) => Some(landed),
            (None, Some(pending)) => Some(pending.await),
            (None, None) => None,
        };
        let Some(next) = next else {
            break;
        };
        if cancelled || !nothing_asked(inbox) {
            for claim in next.0 {
                state.backfill.release(claim);
            }
            break;
        }
        ready = next;
    }
    true
}

type Batch = (
    Vec<postio_sync::backfill::Claim>,
    Option<Rc<RefCell<postio_sync::backfill::Prefetched>>>,
);

/// Awaited outside the drain loop's `select!`, where a stop used to wait out the whole command
/// (#1941);
/// closing the channel fires the claims' token and the prefetch unwinds, returning what it had.
async fn prefetch_watched(
    parts: &EngineParts,
    store: &Store,
    inbox: &async_channel::Receiver<Job>,
    claims: Vec<postio_sync::backfill::Claim>,
) -> Batch {
    let cancel = claims.first().map(|claim| claim.cancel.clone());
    let mut fetching = std::pin::pin!(prefetch_batch(parts, store, claims));
    tokio::select! {
        batch = &mut fetching => batch,
        () = wait_for_close(inbox) => {
            if let Some(cancel) = &cancel {
                cancel.cancel();
            }
            fetching.await
        }
    }
}

/// Best-effort: whatever it does not bring back, `fetch_body` fetches the usual way.
async fn prefetch_batch(
    parts: &EngineParts,
    store: &Store,
    claims: Vec<postio_sync::backfill::Claim>,
) -> Batch {
    let Some(first) = claims.first() else {
        return (claims, None);
    };
    let cancel = first.cancel.clone();
    let requests: Vec<_> = claims.iter().map(|claim| claim.request.clone()).collect();
    let prefetched = match store.connect_background().await {
        Ok(connection) => {
            // Background only if every claim is; one the user or an arrival waits on lifts the
            // whole command.
            let quiet = claims
                .iter()
                .all(|claim| claim.priority == backfill::Priority::Background)
                .then(|| background_backend(parts));
            let cache = backfill::prefetch_text_sections(
                &connection,
                quiet.as_deref().unwrap_or(parts.backend.as_ref()),
                &requests,
                &cancel,
            )
            .await;
            if !cache.is_empty() {
                tracing::debug!(sections = cache.len(), "fetched a batch of text sections");
            }
            Some(Rc::new(RefCell::new(cache)))
        }
        Err(error) => {
            tracing::debug!(%error, "no connection to prefetch with");
            None
        }
    };
    (claims, prefetched)
}

const BODY_WINDOW: usize = 2;

/// Separate so several run on one `FuturesUnordered`; everything touching `State` happens in the
/// drain loop.
async fn fetch_one_body(
    parts: &EngineParts,
    store: &Store,
    inline_cap: Option<u64>,
    prefetched: Option<Rc<RefCell<postio_sync::backfill::Prefetched>>>,
    claim: postio_sync::backfill::Claim,
) -> Settled {
    let settled = |result| Settled {
        message: claim.request.message,
        mailbox: claim.request.mailbox,
        sections: claim.request.want.sections(),
        result,
    };
    // A body queued behind the write gate when shutdown fired has not started and may already be in
    // the
    // prefetch, so nothing below would hear the token.
    if claim.cancel.is_cancelled() {
        return settled(Err(SyncError::Backend(BackendError::Cancelled)));
    }
    let connection = match store.connect_background().await {
        Ok(connection) => connection,
        // Reported settled: a claim nobody finishes is a queue that never drains.
        Err(error) => {
            return settled(Ok(Outcome::Failed {
                reason: error.to_string(),
            }));
        }
    };
    // Lifted out before the fetch with the borrow dropped: the window's fetches share one cache, so
    // a
    // borrow held across the await would panic only under load.
    let mut mine = prefetched.map(|cache| cache.borrow_mut().split_for(&claim.request.remote_id));
    let quiet =
        (claim.priority == backfill::Priority::Background).then(|| background_backend(parts));
    let result = backfill::fetch_body(
        &connection,
        &parts.blobs,
        quiet.as_deref().unwrap_or(parts.backend.as_ref()),
        &claim.request,
        inline_cap,
        mine.as_mut(),
        &claim.cancel,
    )
    .await;
    settled(result)
}

struct Settled {
    message: MessageId,
    mailbox: MailboxId,
    sections: Sections,
    result: Result<Outcome, SyncError>,
}

/// Pushed after every settled body, throttled like `StatusTracker` (250 ms floor): per-body events
/// crowded
/// out the engine's own work. Never dropped: the update that finishes the work, and the first after
/// the
/// queue was empty (seeding, when the denominator becomes known). `now` is a parameter so tests
/// need no sleep (#316).
async fn announce_backfill(parts: &EngineParts, state: &mut State, now: std::time::Instant) {
    let (done, total, drained) = backfill_snapshot(state);

    let opening = state.backfill_announced.is_none();
    let due = state
        .backfill_announced
        .is_none_or(|last| now.duration_since(last) >= BACKFILL_ANNOUNCE_INTERVAL);

    if !(drained || opening || due) {
        return;
    }
    emit_backfill_progress(parts, state, done, total, drained, now).await;
}

const BACKFILL_ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Never throttled: the reading pane's own open must be reported whatever the 250 ms window says
/// (#316).
async fn announce_backfill_now(parts: &EngineParts, state: &mut State, now: std::time::Instant) {
    let (done, total, drained) = backfill_snapshot(state);
    emit_backfill_progress(parts, state, done, total, drained, now).await;
}

fn backfill_snapshot(state: &State) -> (usize, usize, bool) {
    let progress = state.backfill.progress();
    let done = progress.settled();
    let total = done + progress.pending + progress.in_flight;
    let drained = progress.pending == 0 && progress.in_flight == 0;
    (done, total, drained)
}

async fn emit_backfill_progress(
    parts: &EngineParts,
    state: &mut State,
    done: usize,
    total: usize,
    drained: bool,
    now: std::time::Instant,
) {
    // Called every 250 ms during a backfill; the footprint (a checkout and two reads) is recomputed
    // only when the queue opens or drains (#1508).
    if state.footprint.is_none() || drained {
        state.footprint = measure(parts).await;
    }
    // Forget the clock once drained, so the next backfill's first report is prompt.
    state.backfill_announced = if drained { None } else { Some(now) };

    parts.events.emit(Event::BackfillProgress {
        account: parts.account,
        done: done as u32,
        total: total as u32,
        footprint: state.footprint,
    });
}

/// Never fatal: without byte totals the status line falls back to message counts.
async fn measure(parts: &EngineParts) -> Option<MailFootprint> {
    let connection = parts.database.connect().await.ok()?;
    let footprint = MessageRepository::new(&connection)
        .footprint(parts.account)
        .await
        .ok()?;
    Some(MailFootprint {
        total_bytes: footprint.total_bytes,
        attachment_bytes: footprint.attachment_bytes,
        local_bytes: footprint.local_bytes,
        complete: footprint.complete,
    })
}

async fn serve(job: Job, parts: &EngineParts, store: &Store, state: &mut State) {
    match job {
        Job::Drain { reply } => {
            let outcome = drain(parts, store, state).await;
            announce_drain(parts, &outcome).await;
            let _ = reply.send(outcome);
        }
        Job::SeedBackfill {
            mailbox,
            limit,
            reply,
        } => {
            let outcome = match store.connect_background().await {
                Ok(connection) => backfill::seed(&connection, &mut state.backfill, mailbox, limit)
                    .await
                    .map_err(|error| EngineError::new(error.to_string())),
                Err(error) => Err(EngineError::new(error.to_string())),
            };
            // Say it now: seeding is when the denominator becomes known.
            announce_backfill(parts, state, std::time::Instant::now()).await;
            let _ = reply.send(outcome);
        }
        Job::RequestBody { message, reply } => {
            let outcome = match store.connect_background().await {
                Ok(connection) => backfill::request_body(&connection, &mut state.backfill, message)
                    .await
                    .map_err(|error| EngineError::new(error.to_string())),
                Err(error) => Err(EngineError::new(error.to_string())),
            };
            // Unconditional, not throttled: the interactive lane always wins (#316).
            announce_backfill_now(parts, state, std::time::Instant::now()).await;
            let _ = reply.send(outcome);
        }
        Job::RequestPayloads {
            message,
            parts: sections,
            reply,
        } => {
            let outcome = match store.connect_background().await {
                Ok(connection) => {
                    backfill::request_payloads(&connection, &mut state.backfill, message, &sections)
                        .await
                        .map_err(|error| EngineError::new(error.to_string()))
                }
                Err(error) => Err(EngineError::new(error.to_string())),
            }
            .map(|asked| unreachable_unless_up(asked, state.supervisor.link()));
            // Interactive: the user is watching a spinner for these bytes (#316).
            announce_backfill_now(parts, state, std::time::Instant::now()).await;
            let _ = reply.send(outcome);
        }
        Job::RequestWholeMessage { message, reply } => {
            let outcome = match store.connect_background().await {
                Ok(connection) => {
                    backfill::request_whole(&connection, &mut state.backfill, message)
                        .await
                        .map_err(|error| EngineError::new(error.to_string()))
                }
                Err(error) => Err(EngineError::new(error.to_string())),
            };
            announce_backfill_now(parts, state, std::time::Instant::now()).await;
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
            let outcome = sync(parts, store, state, mailbox).await;
            let _ = reply.send(outcome);
        }
    }
}

/// A queued payload ask on a link that cannot carry it says so now, so a person waiting on a save
/// hears "offline" instead of "still downloading" later.
fn unreachable_unless_up(asked: PayloadAsk, link: &Link) -> PayloadAsk {
    let down = match link {
        Link::Online { .. } => false,
        Link::Waiting { attempts, .. } => *attempts > 0,
        Link::Offline | Link::Blocked(_) => true,
    };
    if asked == PayloadAsk::Queued && down {
        PayloadAsk::Unreachable
    } else {
        asked
    }
}

async fn drain(
    parts: &EngineParts,
    store: &Store,
    state: &mut State,
) -> Result<DrainSummary, EngineError> {
    // Without a session the queue is not failed, only not sent yet (local-first): a link that is
    // not up leaves every row where it is.
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
    before_a_drain(state);
    let outcome = run_drain(parts, store).await;
    settle_drain(parts, state, outcome)
}

/// A write `SELECT`s its mailbox, so an `IDLE` armed afterwards misses a delivery that landed just
/// before it;
/// dropping the watcher's verification makes its next step a `STATUS` (#807).
fn before_a_drain(state: &mut State) {
    if let Some(watcher) = state.watcher.as_mut() {
        watcher.unverified();
    }
}

/// Touches nothing of the engine's state, so a wave can run it beside its lanes (#1593); the caller
/// settles with `settle_drain`.
#[tracing::instrument(name = "drain", skip_all)]
async fn run_drain(parts: &EngineParts, store: &Store) -> Result<DrainSummary, SyncError> {
    let connection = store.connect_background().await?;

    let smtp = SmtpContext {
        connector: parts.smtp.as_ref(),
        tokens: parts.tokens.as_ref(),
        blobs: &parts.blobs,
    };
    let drainer = Drainer::with_policy(parts.backend.as_ref(), parts.retry).with_smtp(smtp);

    let report = drainer
        .drain(&connection, parts.account, Utc::now())
        .await?;

    tracing::debug!(
        applied = report.applied,
        coalesced = report.coalesced,
        obsolete = report.obsolete,
        deferred = report.deferred,
        failed = report.failed.len(),
        uncertain = report.uncertain.len(),
        needs_resync = report.needs_resync.len(),
        "drained the operation queue"
    );

    Ok(DrainSummary {
        applied: report.applied,
        coalesced: report.coalesced,
        obsolete: report.obsolete,
        deferred: report.deferred,
        failed: report.failed.iter().map(|failure| failure.said()).collect(),
        uncertain: report
            .uncertain
            .iter()
            .map(|unresolved| unresolved.reason.clone())
            .collect(),
        needs_resync: report.needs_resync.clone(),
        thread_arrivals: report.thread_arrivals.clone(),
        mailboxes_changed: report.mailboxes_changed,
    })
}

fn settle_drain(
    parts: &EngineParts,
    state: &mut State,
    outcome: Result<DrainSummary, SyncError>,
) -> Result<DrainSummary, EngineError> {
    match outcome {
        Ok(summary) => Ok(summary),
        Err(error) => {
            tracing::warn!(%error, "the drain pass failed");
            // Noticed by the operation that hit it, not the next tick: a session that died
            // mid-drain has already cost the user one action.
            if let SyncError::Backend(backend) = &error {
                let moved = state.supervisor.observe(backend, Utc::now());
                announce_link(parts, state, moved);
            }
            Err(EngineError::new(error.to_string()))
        }
    }
}

/// The IMAP-side ceiling: a fourth lane would share a connection and pay a `SELECT` per batch. It
/// does not bind
/// today, because `sync_lanes` is bounded by the smaller database store (#729); #733 measured what
/// raising it buys.
const MAX_SYNC_LANES: usize = 3;

/// One for the UI thread's reads and one for the engine's housekeeping between waves.
const RESERVED_FOR_ELSEWHERE: usize = 2;

/// The database store decides because it is scarcer: passes contend for one writer, and leaving
/// `RESERVED_FOR_ELSEWHERE` keeps the list answering during a first sync. The limit is a parameter
/// so both ceilings are testable.
fn sync_lanes(concurrent_passes: usize) -> usize {
    concurrent_passes
        .saturating_sub(RESERVED_FOR_ELSEWHERE)
        .clamp(1, MAX_SYNC_LANES)
}

fn inbox_pass_pending(state: &State, active: &[MailboxId]) -> bool {
    state
        .to_sync
        .iter()
        .chain(active)
        .any(|mailbox| state.inbox_ids.contains(mailbox))
}

/// INBOX first, and nothing else until it has had its turn (`postio_sync::order`): a queued INBOX
/// is taken ahead of
/// everything, and while its pass or its first page of bodies is in flight nothing else is
/// admitted; if no fetch runs
/// (`bodies_running`) the hold is dropped. `None` means hold the lane free.
fn next_to_sync(
    state: &mut State,
    active: &[MailboxId],
    bodies_running: bool,
) -> Option<MailboxId> {
    if let Some(position) = state
        .to_sync
        .iter()
        .position(|mailbox| state.inbox_ids.contains(mailbox))
    {
        return state.to_sync.remove(position);
    }
    if inbox_pass_pending(state, active) {
        return None;
    }
    if state.inbox_page_pending {
        if bodies_running && !state.backfill.is_idle() {
            return None;
        }
        state.inbox_page_pending = false;
    }
    state.to_sync.pop_front()
}

/// Sync several mailboxes at once, in priority order, until something the wave cannot answer
/// arrives; that job
/// is returned and the interrupted passes go back on the front of the queue. Lanes pay off on slow
/// links (#733).
/// A completed pass admits the next mailbox only after the checks `interruption` watches, or a
/// self-feeding wave
/// starves the arm that yields to the user (#944, #122).
async fn sync_wave(
    parts: &EngineParts,
    store: &Store,
    state: &mut State,
    inbox: &async_channel::Receiver<Job>,
) -> Option<Job> {
    if !state.supervisor.link().is_online() {
        let moved = state
            .supervisor
            .poll(parts.backend.as_ref(), Utc::now(), entropy())
            .await;
        announce_link(parts, state, moved);
    }
    if !state.supervisor.link().is_online() {
        return None;
    }

    // Shared by every pass and cancelled as one; a cancelled pass keeps what it committed, so this
    // costs at most the batch on the wire.
    let cancel = postio_account::cancel::CancelToken::new();

    // Cloned, not borrowed: a pass is settled with `&mut state` while its wave-mates still hold
    // their own handle (see `State::status`).
    let status = state.status.clone();

    // Every pass rings this once per committed batch; the loop answers by looking up from the
    // headers (see `claim_bodies_for_the_wave`).
    let yield_point = tokio::sync::Notify::new();
    let filing = state.filing.get();
    let make_pass = |mailbox: MailboxId, connection: Checkout| {
        let status = status.clone();
        let cancel = cancel.clone();
        let yield_point = &yield_point;
        let filing = filing.clone();
        async move {
            sync_pass(
                parts,
                &connection,
                &status,
                mailbox,
                filing.as_deref(),
                &cancel,
                yield_point,
            )
            .await
        }
    };

    let settle_connection = match store.connect_background().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(%error, "no connection to settle a sync wave");
            return None;
        }
    };

    let lanes = sync_lanes(postio_storage::MAX_CONCURRENT_PASSES);
    let mut admitted: Vec<MailboxId> = Vec::new();
    // Current occupants, unlike `admitted`: a folder that changed while its pass runs needs no
    // second pass; one that changed after does.
    let mut active: Vec<MailboxId> = Vec::new();
    let mut running = FuturesUnordered::new();
    while running.len() < lanes {
        let Some(mailbox) = next_to_sync(state, &active, false) else {
            break;
        };
        match store.connect_background().await {
            Ok(connection) => {
                admitted.push(mailbox);
                active.push(mailbox);
                running.push(make_pass(mailbox, connection));
            }
            Err(error) => {
                // Put it back, not dropped: the next wave will have a connection.
                state.to_sync.push_front(mailbox);
                tracing::warn!(%error, "no connection for a sync lane");
                break;
            }
        }
    }
    if running.is_empty() {
        return None;
    }

    // Settled and pumped the instant each pass finishes (#631); batching waited out the slowest
    // wave-mate.
    let mut interrupted: Vec<MailboxId> = Vec::new();
    let mut asked_to_stop = false;
    let mut handed_back: Option<Job> = None;
    // On a clock of its own, not header commits: a wave whose folders are done keeps fetching
    // bodies with no batch to ring, and new mail went unseen.
    let mut look_at = Some(tokio::time::Instant::now());
    // Beside the lanes, never inside a handler: a pass suspends holding the write gate, so a fetch
    // taking the gate
    // from a handler deadlocks.
    let mut bodies = FuturesUnordered::new();
    let mut looks: FuturesUnordered<futures_util::future::LocalBoxFuture<'_, Look>> =
        FuturesUnordered::new();
    let mut idle_cancel: Option<postio_account::cancel::CancelToken> = None;
    let mut drains = FuturesUnordered::new();
    let make_drain = || run_drain(parts, store);
    let make_bodies = |claims: Vec<postio_sync::backfill::Claim>, inline_cap: Option<u64>| async move {
        fetch_bodies(parts, store, inline_cap, claims, inbox).await
    };
    let make_look = |step: Watch| -> Option<futures_util::future::LocalBoxFuture<'_, Look>> {
        match step {
            Watch::Idle {
                mailbox,
                path,
                timeout,
                cancel,
            } => Some(Box::pin(async move {
                Look::Woke(mailbox, parts.backend.idle(&path, timeout, &cancel).await)
            })),
            Watch::Poll { mailbox, path } => Some(Box::pin(async move {
                Look::Observed(mailbox, parts.backend.status(&path).await)
            })),
            Watch::Wait { .. } => None,
        }
    };
    // A macro, not a closure: each arm borrows a different queue, and a closure would hold them all
    // across the `select!`.
    macro_rules! act_on {
        ($attention:expr) => {
            match $attention {
                Asked::Nothing => {}
                Asked::FetchNow => {
                    let mut claims = Vec::new();
                    while let Some(claim) = state.backfill.next_interactive_body() {
                        claims.push(claim);
                    }
                    if !claims.is_empty() {
                        let inline_cap = state.backfill.policy().max_inline_bytes;
                        bodies.push(make_bodies(claims, inline_cap));
                    }
                }
                Asked::DrainNow => {
                    before_a_drain(state);
                    drains.push(make_drain());
                }
                Asked::Stop(job) => {
                    handed_back = job;
                    asked_to_stop = true;
                    cancel.cancel();
                }
            }
        };
    }

    macro_rules! refill {
        () => {
            while running.len() < lanes {
                let Some(mailbox) = next_to_sync(state, &active, !bodies.is_empty()) else {
                    break;
                };
                match store.connect_background().await {
                    Ok(connection) => {
                        admitted.push(mailbox);
                        active.push(mailbox);
                        running.push(make_pass(mailbox, connection));
                    }
                    Err(error) => {
                        state.to_sync.push_front(mailbox);
                        tracing::warn!(%error, "no connection for a refilled sync lane");
                        break;
                    }
                }
            }
        };
    }

    // `looks` does not keep the wave going: an `IDLE` lasts minutes and is wound down below.
    while !running.is_empty() || !bodies.is_empty() || !drains.is_empty() {
        tokio::select! {
            biased;
            // Completions first, so a finishing wave does not notice a job and mark everything
            // cancelled.
            Some(outcome) = running.next(), if !running.is_empty() => {
                let stopped_early = outcome.was_cancelled();
                // A pass the store turned away as busy is owed another turn like a cancelled one
                // (#1594): requeued at the front, not shown as an error.
                let turned_away = outcome.was_transient();
                if stopped_early || turned_away {
                    interrupted.push(outcome.mailbox);
                }
                active.retain(|mailbox| *mailbox != outcome.mailbox);
                // The wave acts on its own initiative, so a failed pass is reported here; an
                // interrupted one is
                // not a failure and would put a spurious error up.
                if let Err(error) = settle_pass(parts, state, &settle_connection, outcome).await
                    && !stopped_early
                    && !turned_away
                {
                    parts.events.emit(Event::Error {
                        message: error.message().to_string(),
                        account: Some(parts.account),
                    });
                }
                if !asked_to_stop {
                    act_on!(Asked::FetchNow);
                }
                // The interruption conditions are checked here too: `biased;` polls completions
                // first, so
                // back-to-back completions could starve the other arms (the first refill was
                // reverted for it).
                if !asked_to_stop {
                    let draining = !drains.is_empty();
                    act_on!(attend(parts, store, state, inbox, draining).await);
                }
                // Bodies start now beside the lanes, not in place (an in-handler drain froze the
                // lanes and could take the
                // write gate from a holder, #631). Claimed before the refill, which asks whether
                // INBOX's page is on the wire.
                while !asked_to_stop
                    && bodies.len() < BATCHES_BESIDE_A_WAVE
                    && let Some(claims) = claim_bodies_for_the_wave(parts, store, state, inbox, &active).await
                {
                    let inline_cap = state.backfill.policy().max_inline_bytes;
                    bodies.push(make_bodies(claims, inline_cap));
                }
                if !asked_to_stop {
                    refill!();
                }
            }
            // A local mutation is not a job, so the queue is watched too or a send waits out every
            // folder in
            // `to_sync` (#944); off while a drain is in flight so a queue being drained does not
            // start another.
            _ = wait_for_job(inbox), if !asked_to_stop => {
                let draining = !drains.is_empty();
                act_on!(attend(parts, store, state, inbox, draining).await);
            }
            _ = wait_for_queued_work(parts, store), if !asked_to_stop && drains.is_empty() => {
                act_on!(attend(parts, store, state, inbox, false).await);
            }
            Some(fetched) = bodies.next(), if !bodies.is_empty() => {
                settle_bodies(parts, state, fetched).await;
                if !asked_to_stop {
                    act_on!(Asked::FetchNow);
                }
                while !asked_to_stop
                    && bodies.len() < BATCHES_BESIDE_A_WAVE
                    && let Some(claims) = claim_bodies_for_the_wave(parts, store, state, inbox, &active).await
                {
                    let inline_cap = state.backfill.policy().max_inline_bytes;
                    bodies.push(make_bodies(claims, inline_cap));
                }
                if !asked_to_stop {
                    refill!();
                }
            }
            // A failed drain stops the wave: the loop's drain has the link's supervision, and a
            // still-due queue would otherwise start one every poll.
            Some(outcome) = drains.next(), if !drains.is_empty() => {
                let outcome = settle_drain(parts, state, outcome);
                let failed = outcome.is_err();
                announce_drain(parts, &outcome).await;
                if failed && !asked_to_stop {
                    asked_to_stop = true;
                    cancel.cancel();
                }
            }
            // A change to a folder no lane is syncing gets a lane of its own, not by cancelling (a
            // half-done pass costs
            // its resume); a mid-pass folder is queued for one more pass. The watcher's first look
            // always reads as a
            // change, so the inbox gets one pass it did not need, which also catches mail that
            // arrived during its own pass.
            Some(look) = looks.next(), if !looks.is_empty() => {
                idle_cancel = None;
                look_at = Some(tokio::time::Instant::now());
                if let Some(mailbox) = settle_look(parts, state, look)
                    && !asked_to_stop
                {
                    if active.contains(&mailbox) {
                        if !state.to_sync.contains(&mailbox) {
                            state.to_sync.push_front(mailbox);
                        }
                    } else {
                        state.to_sync.retain(|queued| *queued != mailbox);
                        match store.connect_background().await {
                            Ok(connection) => {
                                tracing::debug!(
                                    mailbox = mailbox.get(),
                                    "a watched folder changed during the wave; syncing it beside the lanes"
                                );
                                admitted.push(mailbox);
                                active.push(mailbox);
                                running.push(make_pass(mailbox, connection));
                            }
                            Err(error) => {
                                state.to_sync.push_front(mailbox);
                                tracing::warn!(%error, "no connection for a changed folder's lane");
                            }
                        }
                    }
                }
            }
            () = sleep_until_look(look_at), if look_at.is_some() && looks.is_empty() && !asked_to_stop => {
                look_at = None;
                let step = state
                    .watcher
                    .as_mut()
                    .map_or(Watch::Wait { until: None }, |watcher| watcher.next_push(Utc::now()));
                match step {
                    Watch::Wait { until } => {
                        let delay = until
                            .map(|at| (at - Utc::now()).to_std().unwrap_or_default())
                            .unwrap_or(POLL_INTERVAL);
                        look_at = Some(
                            tokio::time::Instant::now() + delay.clamp(WATCH_FLOOR, POLL_INTERVAL),
                        );
                    }
                    step => {
                        if let Watch::Idle { cancel, .. } = &step {
                            idle_cancel = Some(cancel.clone());
                        }
                        looks.extend(make_look(step));
                    }
                }
            }
            _ = yield_point.notified(), if !asked_to_stop => {
                while bodies.len() < BATCHES_BESIDE_A_WAVE
                    && let Some(claims) = claim_bodies_for_the_wave(parts, store, state, inbox, &active).await
                {
                    let inline_cap = state.backfill.policy().max_inline_bytes;
                    bodies.push(make_bodies(claims, inline_cap));
                }
            }
        }
    }

    // Wound down, never dropped: a dropped `IDLE` hands the pool a session still inside the command
    // (#755).
    if let Some(cancel) = idle_cancel.take() {
        cancel.cancel();
    }
    while let Some(look) = looks.next().await {
        if let Some(mailbox) = settle_look(parts, state, look)
            && !state.to_sync.contains(&mailbox)
        {
            state.to_sync.push_back(mailbox);
        }
    }

    // Back on the front in the order taken off: INBOX must not come back behind the archive it
    // started with.
    for mailbox in admitted.iter().rev() {
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
    handed_back
}

/// `None` when nothing is to fetch or something outranks it. A wave lasts as long as its largest
/// folder, so
/// without this no body is fetched and nothing watches for mail for the hour an archive takes.
async fn claim_bodies_for_the_wave(
    parts: &EngineParts,
    store: &Store,
    state: &mut State,
    inbox: &async_channel::Receiver<Job>,
    active: &[MailboxId],
) -> Option<Vec<postio_sync::backfill::Claim>> {
    if !nothing_asked(inbox)
        || !state.supervisor.link().is_online()
        || has_queued_work(parts, store).await
        // INBOX's headers come before anyone's bodies: a body fetch shares the wire and the store
        // writer with the header pass.
        || inbox_pass_pending(state, active)
    {
        return None;
    }
    // While INBOX's page is the only thing queued, finish it and nothing past: a top-up would seed
    // INBOX's next page and the other folders would wait.
    if state.backfill.is_idle()
        && !state.inbox_page_pending
        && top_up_backfill(parts, store, state).await == 0
    {
        return None;
    }
    let claims = state.backfill.next_batch();
    (!claims.is_empty()).then_some(claims)
}

fn background_backend(parts: &EngineParts) -> Arc<dyn MailBackend> {
    parts
        .backend
        .for_background()
        .unwrap_or_else(|| Arc::clone(&parts.backend))
}

/// The wave looks at the inbox on its own clock and holds a real `IDLE` on the watch connection.
/// Only the inbox:
/// every folder's first observation reads as a change, so polling them all would start a pass per
/// folder.
enum Look {
    Woke(
        MailboxId,
        Result<Vec<postio_account::backend::MailboxEvent>, BackendError>,
    ),
    Observed(
        MailboxId,
        Result<postio_account::backend::MailboxStatus, BackendError>,
    ),
}

fn settle_look(parts: &EngineParts, state: &mut State, look: Look) -> Option<MailboxId> {
    let now = Utc::now();
    let (mailbox, wake) = match look {
        Look::Woke(mailbox, Ok(events)) => {
            (mailbox, state.watcher.as_mut()?.woke(mailbox, &events, now))
        }
        Look::Observed(mailbox, Ok(status)) => (
            mailbox,
            state.watcher.as_mut()?.observed(mailbox, &status, now),
        ),
        Look::Woke(mailbox, Err(error)) | Look::Observed(mailbox, Err(error)) => {
            if let Some(watcher) = state.watcher.as_mut() {
                watcher.failed(mailbox, now);
            }
            let moved = state.supervisor.observe(&error, now);
            announce_link(parts, state, moved);
            return None;
        }
    };
    (wake == Wake::Changed).then_some(mailbox)
}

async fn sleep_until_look(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn fetch_bodies(
    parts: &EngineParts,
    store: &Store,
    inline_cap: Option<u64>,
    claims: Vec<postio_sync::backfill::Claim>,
    inbox: &async_channel::Receiver<Job>,
) -> Fetched {
    let mut fetched = Fetched::default();
    if claims.is_empty() {
        return fetched;
    }
    let (claims, prefetched) = prefetch_watched(parts, store, inbox, claims).await;
    let cancel = claims.first().map(|claim| claim.cancel.clone());
    let mut waiting: std::collections::VecDeque<_> = claims.into();
    let mut running = FuturesUnordered::new();
    let mut cancelled = false;
    loop {
        // A job arriving mid-batch waits only for bodies on the wire; what has not started goes
        // back.
        while running.len() < BODIES_BESIDE_A_WAVE && nothing_asked(inbox) {
            let Some(claim) = waiting.pop_front() else {
                break;
            };
            running.push(fetch_one_body(
                parts,
                store,
                inline_cap,
                prefetched.clone(),
                claim,
            ));
        }
        // Shutdown must not wait for bodies on the wire: they unwind on the token, unfetched ones
        // stay
        // `body_state <> 'full'`, and a write already started finishes.
        let settled = tokio::select! {
            biased;
            next = running.next() => next,
            () = wait_for_close(inbox), if !cancelled => {
                if let Some(cancel) = &cancel {
                    cancel.cancel();
                }
                cancelled = true;
                continue;
            }
        };
        let Some(settled) = settled else {
            break;
        };
        fetched.results.push(settled);
    }
    fetched.unstarted = waiting.into();
    fetched
}

#[derive(Default)]
struct Fetched {
    results: Vec<Settled>,
    unstarted: Vec<postio_sync::backfill::Claim>,
}

async fn settle_bodies(parts: &EngineParts, state: &mut State, fetched: Fetched) {
    for claim in fetched.unstarted {
        state.backfill.release(claim);
    }
    for settled in fetched.results {
        settle_body(
            parts,
            state,
            settled.message,
            settled.mailbox,
            settled.sections,
            settled.result,
        )
        .await;
    }
}

/// An error is not necessarily about the message: a dropped link, timeout or cancel is
/// `Outcome::Interrupted`,
/// retried after a backoff (#1985). A moved `UIDVALIDITY` queues the resync that rebuilds the UIDs.
async fn settle_body(
    parts: &EngineParts,
    state: &mut State,
    message: MessageId,
    mailbox: MailboxId,
    sections: Sections,
    result: Result<Outcome, SyncError>,
) {
    let outcome = result.unwrap_or_else(|error| {
        if let SyncError::Backend(backend) = &error {
            let moved = state.supervisor.observe(backend, Utc::now());
            announce_link(parts, state, moved);
            if backend.requires_full_resync() && !state.to_sync.contains(&mailbox) {
                tracing::debug!(
                    mailbox = mailbox.get(),
                    "a body fetch found the mailbox renumbered; queueing its resync"
                );
                state.to_sync.push_back(mailbox);
            }
        }
        Outcome::Interrupted {
            reason: error.to_string(),
        }
    });
    // Named by section, so a wait for one part hears its own (#1996, #1690).
    if matches!(outcome, Outcome::Stored { .. }) {
        parts.events.emit(Event::BodyLoaded {
            account: parts.account,
            message,
            sections: sections.clone(),
        });
    }
    // A refusal changes the open message's plate (#1918); an interruption tells the host the ask is
    // over, so opening again asks again (#1985).
    if matches!(
        outcome,
        Outcome::Failed { .. } | Outcome::Interrupted { .. }
    ) {
        parts.events.emit(Event::BodyFailed {
            account: parts.account,
            message,
            sections,
        });
    }
    state.backfill.finished(message, outcome);
    announce_backfill(parts, state, std::time::Instant::now()).await;
}

struct PassOutcome {
    mailbox: MailboxId,
    result: Result<SyncSummary, PassFailure>,
    arrived: Vec<MessageId>,
}

impl PassOutcome {
    fn was_cancelled(&self) -> bool {
        matches!(
            &self.result,
            Err(PassFailure::Failed(SyncError::Backend(
                BackendError::Cancelled
            )))
        )
    }

    /// The store turned a write away as busy (#1594): owed another turn like a cancelled pass.
    fn was_transient(&self) -> bool {
        matches!(
            &self.result,
            Err(PassFailure::Failed(SyncError::Storage(error))) if error.is_busy()
        )
    }
}

#[cfg(test)]
mod pass_outcome_tests {
    use super::*;

    fn outcome(result: Result<SyncSummary, PassFailure>) -> PassOutcome {
        PassOutcome {
            mailbox: MailboxId::new(1),
            result,
            arrived: Vec::new(),
        }
    }

    #[test]
    fn a_pass_the_store_turned_away_as_busy_is_transient() {
        let busy = outcome(Err(PassFailure::Failed(SyncError::Storage(
            postio_storage::test_support::busy(),
        ))));
        assert!(busy.was_transient());
        assert!(!busy.was_cancelled());
    }

    #[test]
    fn a_cancelled_a_failed_and_a_finished_pass_are_not() {
        let cancelled = outcome(Err(PassFailure::Failed(SyncError::Backend(
            BackendError::Cancelled,
        ))));
        assert!(!cancelled.was_transient());
        assert!(!outcome(Err(PassFailure::NoSuchMailbox)).was_transient());
        assert!(!outcome(Ok(SyncSummary::default())).was_transient());
    }
}

enum PassFailure {
    /// Renamed or removed by another client between discovery and this wave; not the link's fault.
    NoSuchMailbox,
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

/// First time `sync_mailbox`, afterwards `resync_mailbox`, decided by whether the mailbox has sync
/// state.
/// Touches nothing needing `&mut State`, since passes run concurrently; the rest is `settle_pass`'s
/// job.
// Named `sync`: the span is the log surface for a mailbox pass and `logging_privacy` keys on it.
#[tracing::instrument(name = "sync", skip_all, fields(mailbox = mailbox.get(), path, incremental))]
async fn sync_pass(
    parts: &EngineParts,
    connection: &Checkout,
    status: &RefCell<StatusTracker>,
    mailbox: MailboxId,
    filing: Option<&dyn FilingPass>,
    cancel: &postio_account::cancel::CancelToken,
    yield_point: &tokio::sync::Notify,
) -> PassOutcome {
    let failed = |failure: PassFailure| PassOutcome {
        mailbox,
        result: Err(failure),
        arrived: Vec::new(),
    };
    let record = match MailboxRepository::new(connection).get(mailbox).await {
        Ok(Some(record)) => record,
        Ok(None) => return failed(PassFailure::NoSuchMailbox),
        Err(error) => return failed(PassFailure::Failed(error.into())),
    };
    let synced_before = match SyncStateRepository::new(connection).get(mailbox).await {
        Ok(state) => state.is_some(),
        Err(error) => return failed(PassFailure::Failed(error.into())),
    };

    // A folder path is a name the user chose, not mail or an address. INBOX is what a person waits
    // on;
    // every other folder's headers are a backfill and take a shared connection only when nothing
    // else wants it.
    let quiet =
        (record.role != postio_model::MailboxRole::Inbox).then(|| background_backend(parts));
    let backend = quiet.as_deref().unwrap_or(parts.backend.as_ref());
    tracing::Span::current().record("path", record.path.as_str());
    tracing::Span::current().record("incremental", synced_before);
    tracing::info!("sync started");
    let started = std::time::Instant::now();

    announce_status(parts, &status.borrow_mut().on_sync_started(mailbox));

    let mut committed = Committed {
        parts,
        status,
        repaint: Repaint::new(parts.events.clone(), parts.account, mailbox),
        yield_point,
    };
    // Filled only by the incremental branch: a first sync inserts thousands of messages new to this
    // store, not new mail (no notification storm).
    let mut arrived: Vec<MessageId> = Vec::new();
    // `SyncSummary` folds changed and vanished into `updated`, so only here can we tell that
    // `arrived` accounts for everything the pass did to the list.
    let mut only_arrivals = false;
    let mut thread_arrivals = postio_sync::threaded::ThreadArrivals::default();
    let result = if synced_before {
        // Arrivals go to the filing pass when a Focus host gave one; a first sync never files (spec
        // 007).
        resync::resync_mailbox_filing(connection, backend, &record, filing, cancel, |progress| {
            committed.batch(progress)
        })
        .await
        .map(|outcome| {
            if let resync::Outcome::Incremental {
                arrived: ids,
                changed,
                vanished,
                filed_away,
                thread_arrivals: joined,
            } = &outcome
            {
                thread_arrivals = joined.clone();
                // What Focus's filing pass filtered or held is not new mail anybody is told of; the
                // view re-reads instead.
                arrived = ids
                    .iter()
                    .copied()
                    .filter(|id| !filed_away.contains(id))
                    .collect();
                only_arrivals = *vanished == 0 && *changed == ids.len() && filed_away.is_empty();
            }
            // A backend with no MODSEQ re-reads the folder every pass, so what it inserted arrived
            // since the last (#1704);
            // every other full pass is a reload, and announcing it would be a notification storm.
            if let resync::Outcome::Full {
                reason: postio_model::FullResyncReason::NoModSeq,
                report,
            } = &outcome
            {
                arrived = report
                    .arrived
                    .iter()
                    .copied()
                    .filter(|id| !report.filed_away.contains(id))
                    .collect();
                thread_arrivals = report.thread_arrivals.clone();
            }
            summarise_resync(outcome)
        })
    } else {
        initial::sync_mailbox(connection, backend, &record, cancel, |progress| {
            committed.batch(progress)
        })
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
            elapsed_ms = started.elapsed().as_millis() as u64,
            "sync finished"
        );
        // Recorded here, where a pass has completed: a failed pass has not synced the folder
        // (#1281).
        if let Err(error) = MailboxRepository::new(connection)
            .record_sync(mailbox, chrono::Utc::now())
            .await
        {
            tracing::warn!(%error, "the folder synced but the time could not be recorded");
        }
        // `MessageListChanged` makes the view discard and rebuild every row widget; when `arrived`
        // describes every
        // insert, `NewMail` lets it prepend instead, and emitting both caused the flicker on every
        // sync tick (#72).
        let described_by_arrivals = only_arrivals && !arrived.is_empty();
        if summary.changed() && !described_by_arrivals {
            parts.events.emit(Event::MessageListChanged {
                account: parts.account,
                mailbox,
            });
        }
        if !arrived.is_empty() {
            parts.events.emit(Event::NewMail {
                account: parts.account,
                mailbox,
                messages: arrived.clone(),
            });
        }
        // After the transactions commit, one event per thread; filed-away arrivals count, since the
        // thread changed.
        for (thread, members_added) in thread_arrivals.into_threads() {
            parts.events.emit(Event::ThreadChanged {
                account: parts.account,
                thread,
                members_added,
            });
        }
    }

    PassOutcome {
        mailbox,
        result: result.map_err(PassFailure::Failed),
        arrived,
    }
}

async fn settle_pass(
    parts: &EngineParts,
    state: &mut State,
    connection: &Checkout,
    outcome: PassOutcome,
) -> Result<SyncSummary, EngineError> {
    let turned_away = outcome.was_transient();
    let PassOutcome {
        mailbox,
        result,
        arrived,
    } = outcome;
    let now = Utc::now();

    // A cancelled pass is neither a failure nor a completed one: it keeps what it committed and
    // leaves the supervisor alone, since the link is fine.
    if let Err(PassFailure::Failed(SyncError::Backend(BackendError::Cancelled))) = &result {
        announce_status(parts, &state.status.borrow_mut().on_sync_abandoned(mailbox));
        return Err(EngineError::new("the sync pass was interrupted"));
    }

    match result {
        Ok(summary) => {
            announce_status(
                parts,
                &state.status.borrow_mut().on_sync_finished(mailbox, now),
            );
            if state.failing.remove(&mailbox) {
                parts.events.emit(Event::MailboxSyncChanged {
                    account: parts.account,
                    mailbox,
                    failure: None,
                });
            }
            // `done == total` is how the stream says a sync completed; an empty mailbox never
            // counted anything to reach it.
            if matches!(
                state.status.borrow().status(),
                SyncStatus::Idle {
                    last_sync: Some(_),
                    ..
                }
            ) {
                parts.events.emit(Event::SyncProgress {
                    account: parts.account,
                    done: 0,
                    total: 0,
                });
            }
            if summary.changed() {
                // A sync changes the set of messages missing a body, and new mail arrives above
                // wherever the backwards walk had reached.
                state.backfill_covered = false;
                match backfill::seed(
                    connection,
                    &mut state.backfill,
                    mailbox,
                    parts.backfill.seed_batch,
                )
                .await
                {
                    Ok(queued) => {
                        if queued > 0 && state.inbox_ids.contains(&mailbox) {
                            state.inbox_page_pending = true;
                        }
                    }
                    Err(error) => {
                        parts.events.emit(Event::Error {
                            message: error.to_string(),
                            account: Some(parts.account),
                        });
                    }
                }

                // What just arrived goes ahead of the backlog: words, then attachments (see
                // `Backfill::arrived`).
                for message in &arrived {
                    if let Err(error) =
                        backfill::request_arrived(connection, &mut state.backfill, *message).await
                    {
                        tracing::debug!(%error, "an arrival could not be queued for its body");
                    }
                }

                // The draft reserved its `Message-ID` before the first attempt, so a message
                // arriving with that id
                // confirms the send (#674); only when the pass changed something.
                match postio_sync::send::confirm_unconfirmed(connection, parts.account).await {
                    Ok(resolved) => {
                        for (draft, message) in resolved {
                            tracing::debug!(draft = draft.get(), "an unconfirmed send is resolved");
                            parts.events.emit(Event::MessagesChanged {
                                account: parts.account,
                                messages: vec![message],
                            });
                        }
                    }
                    // The drafts stay `Unconfirmed`, the honest state; the next pass looks again.
                    Err(error) => {
                        tracing::warn!(%error, "could not check for resolved sends");
                    }
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
            announce_status(parts, &state.status.borrow_mut().on_sync_abandoned(mailbox));
            // Kept until a pass of the folder completes (#1731), unless the link went down (its
            // state says so) or nothing failed the folder.
            let the_folders = !turned_away && !matches!(failure, PassFailure::NoSuchMailbox);
            if the_folders && state.supervisor.link().is_online() {
                state.failing.insert(mailbox);
                parts.events.emit(Event::MailboxSyncChanged {
                    account: parts.account,
                    mailbox,
                    failure: Some(failure.to_string()),
                });
            }
            Err(EngineError::new(failure.to_string()))
        }
    }
}

/// One mailbox synced now for a caller waiting on the answer (`Job::Sync`), without the wave's
/// machinery.
async fn sync(
    parts: &EngineParts,
    store: &Store,
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

    let connection = store
        .connect()
        .await
        .map_err(|error| EngineError::new(error.to_string()))?;
    let cancel = postio_account::cancel::CancelToken::new();
    let yield_point = tokio::sync::Notify::new();
    let filing = state.filing.get();
    let outcome = sync_pass(
        parts,
        &connection,
        &state.status,
        mailbox,
        filing.as_deref(),
        &cancel,
        &yield_point,
    )
    .await;
    settle_pass(parts, state, &connection, outcome).await
}

fn summarise_resync(outcome: resync::Outcome) -> SyncSummary {
    match outcome {
        resync::Outcome::UpToDate => SyncSummary::default(),
        resync::Outcome::Full { report, .. } | resync::Outcome::Rebuilt { report } => SyncSummary {
            inserted: report.inserted,
            updated: report.updated,
            threaded: report.threaded,
            full: true,
        },
        // An incremental pull counts what moved, not what it wrote; a flag change or a vanished
        // message makes the list stale.
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

fn offline_reason(link: &Link) -> String {
    match link {
        Link::Blocked(blocker) => blocker.reason().to_string(),
        Link::Offline => "there is no network".to_string(),
        Link::Waiting { .. } => "not connected yet".to_string(),
        Link::Online { .. } => "connected".to_string(),
    }
}

/// One place because a link change and a sync pass move the same status. `SyncProgress` rather than
/// `ConnectionChanged` while counting: different questions, drawn differently.
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
    if let SyncStatus::Error { reason, .. } = status {
        parts.events.emit(Event::Error {
            message: reason.clone(),
            account: Some(parts.account),
        });
    }
}

/// `needs_credentials` is how `postio-sync` tells its two blockers apart: a refused credential is
/// `Auth` (never retried
/// on a timer), anything else a retry cannot fix is `Config`. `Network` and `Server` are reserved
/// (ADR 0005 Q10).
fn connection_of(status: &SyncStatus) -> postio_core::ConnectionState {
    match status {
        SyncStatus::Offline => postio_core::ConnectionState::Offline,
        SyncStatus::Connecting => postio_core::ConnectionState::Connecting,
        SyncStatus::Idle { .. } | SyncStatus::Syncing { .. } => {
            postio_core::ConnectionState::Online
        }
        SyncStatus::Error {
            needs_credentials,
            no_password,
            ..
        } => postio_core::ConnectionState::Failing {
            reason: if *no_password {
                postio_core::FailureReason::NoPassword
            } else if *needs_credentials {
                postio_core::FailureReason::Auth
            } else {
                postio_core::FailureReason::Config
            },
        },
    }
}

fn announce_link(parts: &EngineParts, state: &mut State, moved: Option<Link>) {
    let Some(link) = moved else {
        return;
    };
    // Through `Debug`, then redacted: a `Blocked` link carries its reason, and an unusable
    // credential names the account.
    tracing::info!(
        link = %postio_model::address::redact_addresses(&format!("{link:?}")),
        "connection state changed"
    );
    let status = state.status.borrow_mut().on_link(&link);
    parts.events.emit(Event::ConnectionChanged {
        account: parts.account,
        state: connection_of(&status),
    });
    if let SyncStatus::Error { reason, .. } = &status {
        tracing::error!(
            reason = %postio_model::address::redact_addresses(reason),
            "connection failing"
        );
        parts.events.emit(Event::Error {
            message: reason.clone(),
            account: Some(parts.account),
        });
    }
}

/// `None` before the first sync has found one.
async fn drafts_mailbox(parts: &EngineParts) -> Option<MailboxId> {
    let connection = parts.database.connect().await.ok()?;
    postio_storage::repository::MailboxRepository::new(&connection)
        .by_role(parts.account, postio_model::MailboxRole::Drafts)
        .await
        .ok()
        .flatten()
        .map(|mailbox| mailbox.id)
}

async fn announce_drain(parts: &EngineParts, outcome: &Result<DrainSummary, EngineError>) {
    let events = &parts.events;
    let account = parts.account;
    match outcome {
        Ok(summary) => {
            // A drain moves drafts between Drafts and the Outbox, decided by `send_state`, so both
            // are re-read. Coarse on
            // purpose: the report does not distinguish operation kinds, and a stale "Sending" row
            // is worse.
            let moved =
                summary.applied > 0 || !summary.failed.is_empty() || !summary.uncertain.is_empty();
            if moved && let Some(drafts) = drafts_mailbox(parts).await {
                events.emit(Event::MessageListChanged {
                    account,
                    mailbox: drafts,
                });
            }
            for mailbox in &summary.needs_resync {
                events.emit(Event::MessageListChanged {
                    account,
                    mailbox: *mailbox,
                });
            }
            if summary.mailboxes_changed {
                events.emit(Event::MailboxesChanged { account });
            }
            for (thread, members_added) in summary.thread_arrivals.clone().into_threads() {
                events.emit(Event::ThreadChanged {
                    account,
                    thread,
                    members_added,
                });
            }
            // Never silently empty: an operation given up on is one the user
            // believes happened.
            for reason in &summary.failed {
                events.emit(Event::Error {
                    message: reason.clone(),
                    account: Some(account),
                });
            }
            // Deliberately not an error (#674): a send that may have arrived is a question, asked
            // by the `Unconfirmed` draft. Logged so a bug report can see it.
            for reason in &summary.uncertain {
                tracing::warn!(reason, "an operation settled without a knowable outcome");
            }
        }
        Err(error) => {
            events.emit(Event::Error {
                message: error.message().to_string(),
                account: Some(account),
            });
        }
    }
}

/// The clock, not an RNG: this only has to stop clients retrying in lockstep.
fn entropy() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_stall_warning_names_what_the_engine_was_doing() {
        use postio_test_support::logs::Captured;

        let busy = Busy::new();
        busy.set("the body backfill (message 7)");
        let thread = EngineThread {
            handle: Mutex::new(Some(
                std::thread::Builder::new()
                    .spawn(|| std::thread::sleep(Duration::from_millis(200)))
                    .expect("a sleeper thread"),
            )),
            busy,
        };

        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || thread.join_until(Instant::now()));

        let log = String::from_utf8_lossy(&captured.0.lock().expect("not poisoned")).into_owned();
        assert!(
            log.contains("did not stop within"),
            "the expired deadline logged no warning at all:\n{log}"
        );
        assert!(
            log.contains("the body backfill (message 7)"),
            "the warning does not say what was still running:\n{log}"
        );
    }

    /// `sync_lanes` is bounded by the database store, which binds before `MAX_SYNC_LANES` (#729);
    /// fails if the constants move without the docs.
    #[tokio::test]
    async fn the_store_is_what_bounds_the_lanes_not_the_imap_ceiling() {
        let _database = postio_storage::test_support::memory().await;
        assert_eq!(
            sync_lanes(postio_storage::MAX_CONCURRENT_PASSES),
            postio_storage::MAX_CONCURRENT_PASSES - RESERVED_FOR_ELSEWHERE,
            "the reserve against the database store decides, not MAX_SYNC_LANES"
        );
        // Asserted on what the store yields: clippy rejects an assertion whose operands are all
        // constant.
        assert!(
            sync_lanes(postio_storage::MAX_CONCURRENT_PASSES) < MAX_SYNC_LANES,
            "MAX_SYNC_LANES is a ceiling held in reserve: it does not bind on \
             the shipped store, and #733 measured what reaching it would buy"
        );
    }

    /// `MAX_SYNC_LANES` is unreached today, not dead: a larger ceiling reaches it.
    #[test]
    fn a_larger_ceiling_runs_into_max_sync_lanes() {
        assert_eq!(
            sync_lanes(MAX_SYNC_LANES * 4),
            MAX_SYNC_LANES,
            "with passes to spare the IMAP ceiling is what caps a wave"
        );
    }

    #[test]
    fn a_refused_credential_fails_as_auth_and_anything_else_as_config() {
        // ADR 0005 Q10: both blocker cases surface typed rather than as prose to parse.
        let auth = super::connection_of(&postio_sync::SyncStatus::Error {
            reason: "the server refused the password".into(),
            needs_credentials: true,
            no_password: false,
        });
        let config = super::connection_of(&postio_sync::SyncStatus::Error {
            reason: "certificate does not verify".into(),
            needs_credentials: false,
            no_password: false,
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
        assert_eq!(
            super::connection_of(&postio_sync::SyncStatus::Connecting),
            postio_core::ConnectionState::Connecting
        );
    }
    use super::*;
    use postio_core::bridge::event_channel;

    fn empty_state() -> State {
        State {
            busy: Busy::new(),
            backfill: Backfill::new(BackfillPolicy::default()),
            backfill_announced: None,
            footprint: None,
            supervisor: Supervisor::new(ReconnectPolicy::default()),
            status: Rc::new(RefCell::new(StatusTracker::new())),
            online: false,
            to_sync: std::collections::VecDeque::new(),
            inbox_ids: Vec::new(),
            inbox_page_pending: false,
            failing: Default::default(),
            watcher: None,
            backfill_covered: false,
            filing: FilingSlot::default(),
        }
    }

    /// Keeps the receiving end of the event channel so a test can see what was announced.
    async fn parts_over(
        database: Store,
    ) -> (
        EngineParts,
        postio_core::bridge::EventStream,
        tempfile::TempDir,
    ) {
        let connection = database.connect().await.expect("checkout");
        let account = postio_storage::test_support::account(&connection).await;
        drop(connection);
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (sink, events) = postio_core::bridge::event_channel();
        let parts = EngineParts {
            account: account.id,
            database,
            blobs,
            backend: Arc::new(postio_account::backend::MockBackend::new()),
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
                postio_account::secret::MemorySecretStore::default(),
            ))),
            events: sink,
            retry: RetryPolicy::default(),
            backfill: BackfillPolicy::default(),
            reconnect: ReconnectPolicy::default(),
            watch: WatchPolicy::default(),
            network: NetworkSource::Ignored,
            mailbox_roles: Default::default(),
            clock: Arc::new(SystemClock),
        };
        (parts, events, directory)
    }

    #[tokio::test]
    async fn queueing_clears_the_mark_of_a_failed_folder_that_is_gone() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database.clone()).await;
        let connection = database.connect().await.expect("checkout");
        let inbox =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "INBOX")
                .await;
        drop(connection);
        let gone = MailboxId::new(inbox.id.get() + 100);
        let mut state = empty_state();
        state.failing.extend([inbox.id, gone]);

        queue_every_mailbox(&parts, &database, &mut state).await;

        let mut cleared = Vec::new();
        while let Some(event) = events.try_next() {
            if let Event::MailboxSyncChanged {
                mailbox,
                failure: None,
                ..
            } = event
            {
                cleared.push(mailbox);
            }
        }
        assert_eq!(cleared, vec![gone]);
        assert_eq!(
            state.failing.into_iter().collect::<Vec<_>>(),
            vec![inbox.id]
        );
    }

    /// INBOX first, then the folders a person reads next, then the rest, whatever order they were
    /// created in.
    #[tokio::test]
    async fn queueing_every_mailbox_puts_inbox_first_and_orders_the_rest_by_role() {
        let database = postio_storage::test_support::memory().await;
        let (parts, _events, _directory) = parts_over(database.clone()).await;
        let connection = database.connect().await.expect("checkout");

        let archive = postio_storage::test_support::mailbox(
            &connection,
            &account_of(&parts).await,
            "Archive",
        )
        .await;
        let regular = postio_storage::test_support::mailbox(
            &connection,
            &account_of(&parts).await,
            "Projects",
        )
        .await;
        let trash =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "Trash")
                .await;
        let sent =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "Sent")
                .await;
        let inbox =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "INBOX")
                .await;
        let junk =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "Junk")
                .await;
        let drafts =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "Drafts")
                .await;
        let flagged = postio_storage::test_support::mailbox(
            &connection,
            &account_of(&parts).await,
            "Flagged",
        )
        .await;
        drop(connection);

        let mut state = empty_state();
        queue_every_mailbox(&parts, &database, &mut state).await;

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

    async fn account_of(parts: &EngineParts) -> postio_model::Account {
        let connection = parts.database.connect().await.expect("checkout");
        postio_storage::repository::AccountRepository::new(&connection)
            .get(parts.account)
            .await
            .expect("read the test account")
            .expect("the test account exists")
    }

    /// Time is a parameter so the throttle is tested without sleeping.
    #[test]
    fn the_first_batch_is_announced_at_once_and_the_rest_are_throttled() {
        let (sink, events) = event_channel();
        let mut repaint = Repaint::new(sink, AccountId::new(1), MailboxId::new(4));
        let start = Instant::now();

        repaint.batch_committed(start);
        assert_eq!(events.len(), 1, "the first batch has to show immediately");

        // Three hundred batches of an initial sync must not be three hundred reloads.
        for step in 1..=50 {
            let inside = REPAINT_INTERVAL / 100 * (step % 100);
            repaint.batch_committed(start + inside);
        }
        assert_eq!(events.len(), 1, "batches inside the window must coalesce");

        repaint.batch_committed(start + REPAINT_INTERVAL);
        assert_eq!(events.len(), 2);
        repaint.batch_committed(start + REPAINT_INTERVAL * 2);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn a_pass_that_commits_nothing_says_nothing() {
        let (sink, events) = event_channel();
        let _repaint = Repaint::new(sink, AccountId::new(1), MailboxId::new(4));

        assert_eq!(events.len(), 0);
    }

    fn body_request(n: u32) -> backfill::BodyRequest {
        backfill::BodyRequest {
            message: MessageId::new(n as i64),
            mailbox: MailboxId::new(1),
            path: "INBOX".to_string(),
            uid: postio_model::Uid::new(n),
            remote_id: postio_model::RemoteId::new(format!("1:{n}")),
            size: 100,
            text_size: 100,
            rank: 0,
            received_at: Utc::now(),
            want: postio_sync::Want::Text,
        }
    }

    async fn settle_claimed(
        parts: &EngineParts,
        state: &mut State,
        n: u32,
        result: Result<Outcome, SyncError>,
    ) {
        state.backfill.request_now(body_request(n));
        let claim = state.backfill.next_interactive_body().expect("the claim");
        settle_body(
            parts,
            state,
            claim.request.message,
            claim.request.mailbox,
            claim.request.want.sections(),
            result,
        )
        .await;
    }

    #[tokio::test]
    async fn a_body_fetch_that_asks_for_a_full_resync_queues_the_resync_of_its_folder() {
        let database = postio_storage::test_support::memory().await;
        let (parts, _events, _directory) = parts_over(database).await;
        let mut state = empty_state();

        let renumbered = BackendError::ResyncIntegrityLost {
            mailbox: "INBOX".to_owned(),
            skipped: 1,
        };
        settle_claimed(&parts, &mut state, 1, Err(SyncError::Backend(renumbered))).await;

        assert_eq!(state.to_sync, [MailboxId::new(1)]);
    }

    #[tokio::test]
    async fn a_dropped_link_under_a_body_fetch_queues_no_resync_and_says_so_to_the_pane() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database).await;
        let mut state = empty_state();

        settle_claimed(
            &parts,
            &mut state,
            1,
            Err(SyncError::Backend(BackendError::Cancelled)),
        )
        .await;

        assert!(state.to_sync.is_empty());
        let mut told = Vec::new();
        while let Some(event) = events.try_next() {
            told.push(event);
        }
        assert!(
            told.iter().any(|event| matches!(
                event,
                Event::BodyFailed { message, .. } if *message == MessageId::new(1)
            )),
            "the pane reads again"
        );
        // Not set aside: after the backoff the same message is offered again.
        assert!(
            state
                .backfill
                .enqueue_at(body_request(1), Instant::now() + Duration::from_secs(3600)),
            "a dropped link is not the message's fault"
        );
    }

    #[tokio::test]
    async fn a_refused_part_is_said_by_its_section_so_a_text_wait_does_not_hear_it() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database).await;
        let mut state = empty_state();
        let mut request = body_request(1);
        request.want = postio_sync::Want::Payloads(vec!["2".to_owned()]);
        state.backfill.request_now(request);
        let claim = state.backfill.next_interactive_body().expect("the claim");
        settle_body(
            &parts,
            &mut state,
            claim.request.message,
            claim.request.mailbox,
            claim.request.want.sections(),
            Ok(Outcome::Failed {
                reason: "NO [UNAVAILABLE]".to_owned(),
            }),
        )
        .await;

        let mut failed = Vec::new();
        while let Some(event) = events.try_next() {
            if let Event::BodyFailed { sections, .. } = event {
                failed.push(sections);
            }
        }
        assert_eq!(failed, [Sections::Parts(vec!["2".to_owned()])]);
    }

    #[test]
    fn a_queued_ask_on_a_link_that_is_down_says_it_cannot_be_reached() {
        let now = Utc::now();
        let up = Link::Online { since: now };
        let starting = Link::Waiting {
            attempts: 0,
            retry_at: now,
        };
        let failing = Link::Waiting {
            attempts: 2,
            retry_at: now,
        };
        assert_eq!(
            unreachable_unless_up(PayloadAsk::Queued, &up),
            PayloadAsk::Queued
        );
        assert_eq!(
            unreachable_unless_up(PayloadAsk::Queued, &starting),
            PayloadAsk::Queued,
            "a link about to make its first attempt is not down"
        );
        for down in [failing, Link::Offline] {
            assert_eq!(
                unreachable_unless_up(PayloadAsk::Queued, &down),
                PayloadAsk::Unreachable,
                "{down:?}"
            );
        }
        assert_eq!(
            unreachable_unless_up(PayloadAsk::Local, &Link::Offline),
            PayloadAsk::Local,
            "bytes already here are here, offline or not"
        );
    }

    #[tokio::test]
    async fn a_second_announcement_inside_the_floor_is_throttled() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database).await;
        let mut state = empty_state();
        let t0 = Instant::now();

        state.backfill.request_now(body_request(1));
        announce_backfill(&parts, &mut state, t0).await;
        assert_eq!(events.len(), 1, "the first claim must announce at once");

        state.backfill.request_now(body_request(2));
        announce_backfill(&parts, &mut state, t0 + Duration::from_millis(10)).await;
        assert_eq!(
            events.len(),
            1,
            "10ms is well inside the 250ms floor -- a second background-\
             shaped announce this soon must still be throttled"
        );
    }

    #[tokio::test]
    async fn announce_backfill_now_is_never_throttled() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database).await;
        let mut state = empty_state();
        let t0 = Instant::now();

        state.backfill.request_now(body_request(1));
        announce_backfill(&parts, &mut state, t0).await;
        assert_eq!(events.len(), 1);

        state.backfill.request_now(body_request(2));
        announce_backfill_now(&parts, &mut state, t0 + Duration::from_millis(10)).await;

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

    #[tokio::test]
    async fn backfill_progress_carries_what_the_mail_weighs() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database.clone()).await;
        let connection = database.connect().await.expect("checkout");
        let inbox =
            postio_storage::test_support::mailbox(&connection, &account_of(&parts).await, "INBOX")
                .await;

        let repository = MessageRepository::new(&connection);
        let mut message =
            postio_model::Message::new(account_of(&parts).await.id, inbox.id, chrono::Utc::now());
        message.size = 5_000;
        message.server.uid = Some(postio_model::Uid::new(1));
        message.server.remote_id = Some(postio_model::RemoteId::new("1:1"));
        message.attachments = vec![postio_model::Attachment::new(
            postio_model::MessageId::UNASSIGNED,
            "application/pdf",
            4_000,
        )];
        repository.create(&mut message).await.expect("create");
        drop(connection);

        let mut state = empty_state();
        state.backfill.request_now(body_request(1));
        announce_backfill_now(&parts, &mut state, std::time::Instant::now()).await;

        match events.try_next() {
            Some(Event::BackfillProgress { footprint, .. }) => {
                let footprint = footprint.expect("the store could answer");
                assert_eq!(footprint.total_bytes, 5_000);
                assert_eq!(footprint.attachment_bytes, 4_000);
                assert_eq!(footprint.local_bytes, 0, "nothing fetched yet");
                assert!(
                    !footprint.complete,
                    "no folder has finished a header pass, so this is a floor \
                     and the surface must say \"over\""
                );
            }
            other => panic!("expected a BackfillProgress, got {other:?}"),
        }
    }

    /// A drain resets the clock whichever announce function reaches it, so a finished queue does
    /// not stick at `2000 of 2000`.
    #[tokio::test]
    async fn a_drained_queue_resets_the_throttle_clock() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database).await;
        let mut state = empty_state();
        let t0 = Instant::now();

        state.backfill.request_now(body_request(1));
        announce_backfill(&parts, &mut state, t0).await;
        let claim = state.backfill.next_body().expect("the claim just queued");
        state
            .backfill
            .finished(claim.request.message, Outcome::Gone);
        announce_backfill(&parts, &mut state, t0 + Duration::from_millis(10)).await;
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

    #[tokio::test]
    async fn only_a_pass_that_failed_its_folder_marks_it_and_only_a_completed_one_clears_it() {
        let database = postio_storage::test_support::memory().await;
        let (parts, events, _directory) = parts_over(database.clone()).await;
        let mut state = empty_state();
        state
            .supervisor
            .poll(parts.backend.as_ref(), Utc::now(), 0)
            .await;
        assert!(state.supervisor.link().is_online(), "the mock connects");
        let connection = database.connect().await.expect("checkout");
        let mailbox = MailboxId::new(7);
        let end = |result| PassOutcome {
            mailbox,
            result,
            arrived: Vec::new(),
        };
        let marks = |events: &postio_core::bridge::EventStream| {
            let mut marks = Vec::new();
            while let Some(event) = events.try_next() {
                if let Event::MailboxSyncChanged {
                    mailbox: marked,
                    failure,
                    ..
                } = event
                {
                    assert_eq!(marked, mailbox);
                    marks.push(failure);
                }
            }
            marks
        };
        let refused = || {
            Err(PassFailure::Failed(SyncError::Backend(
                BackendError::Rejected {
                    command: "SELECT".to_owned(),
                    reason: "Mailbox is locked".to_owned(),
                },
            )))
        };

        let _ = settle_pass(
            &parts,
            &mut state,
            &connection,
            end(Ok(SyncSummary::default())),
        )
        .await;
        assert_eq!(marks(&events), Vec::<Option<String>>::new());

        for result in [
            Err(PassFailure::Failed(SyncError::Backend(
                BackendError::Cancelled,
            ))),
            Err(PassFailure::Failed(SyncError::Storage(
                postio_storage::test_support::busy(),
            ))),
            Err(PassFailure::NoSuchMailbox),
        ] {
            let _ = settle_pass(&parts, &mut state, &connection, end(result)).await;
        }
        assert_eq!(marks(&events), Vec::<Option<String>>::new());

        let _ = settle_pass(&parts, &mut state, &connection, end(refused())).await;
        let _ = settle_pass(&parts, &mut state, &connection, end(refused())).await;
        let marked = marks(&events);
        assert_eq!(marked.len(), 2, "{marked:?}");
        assert!(
            marked.iter().all(|mark| mark
                .as_deref()
                .is_some_and(|why| why.contains("Mailbox is locked"))),
            "{marked:?}"
        );
        assert!(
            state.supervisor.link().is_online(),
            "a refused SELECT keeps the link"
        );
        let _ = settle_pass(
            &parts,
            &mut state,
            &connection,
            end(Err(PassFailure::Failed(SyncError::Backend(
                BackendError::Cancelled,
            )))),
        )
        .await;
        assert_eq!(marks(&events), Vec::<Option<String>>::new());
        let _ = settle_pass(
            &parts,
            &mut state,
            &connection,
            end(Ok(SyncSummary::default())),
        )
        .await;
        let _ = settle_pass(
            &parts,
            &mut state,
            &connection,
            end(Ok(SyncSummary::default())),
        )
        .await;
        assert_eq!(marks(&events), vec![None]);
    }
}
