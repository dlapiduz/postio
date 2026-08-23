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

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use postio_imap::backend::MailBackend;
use postio_imap::secret::SecretStore;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_smtp::transport::SmtpConnector;
use postio_storage::{BlobStore, Database, Pool};
use postio_sync::status::StatusTracker;
use postio_sync::{
    Backfill, BackfillPolicy, BackfillProgress, Drainer, ReconnectPolicy, RetryPolicy, SmtpContext,
    Supervisor, SyncError, SyncStatus, backfill,
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
            parts.events.emit(Event::Error {
                message: format!("the sync engine has no runtime: {error}"),
            });
            return;
        }
    };

    runtime.block_on(async move {
        let mut state = State {
            backfill: Backfill::new(parts.backfill),
            supervisor: Supervisor::new(parts.reconnect),
            status: StatusTracker::new(),
            online: false,
        };
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        // The first tick fires immediately; skipping it would leave the link
        // unexamined until the first interval elapsed.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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
            }

            // Whatever the link just did, act on it before anything else: a
            // queue that has been waiting for a connection should go out the
            // moment there is one, not on the next thing the user happens to
            // do.
            if came_up(&mut state) {
                let outcome = drain(&parts, &pool, &mut state).await;
                announce_drain(&parts.events, &outcome);
            }

            // Then fetch bodies, but only while nothing else is asking. One
            // at a time and the inbox checked between each, so the longest a
            // user waits behind the backfill is the one body already on the
            // wire.
            while inbox.is_empty()
                && state.supervisor.link().is_online()
                && pump_body(&parts, &pool, &mut state).await
            {}
        }
    });
}

/// What the engine's thread keeps between jobs.
struct State {
    backfill: Backfill,
    supervisor: Supervisor,
    status: StatusTracker,
    /// Whether the link was up last time anything looked.
    ///
    /// Only the *transition* is interesting: a connection that has been up
    /// for an hour is not a reason to drain again.
    online: bool,
}

/// Whether the link has come up since this was last asked.
fn came_up(state: &mut State) -> bool {
    let online = state.supervisor.link().is_online();
    let transition = online && !state.online;
    state.online = online;
    transition
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
        parts.events.emit(Event::BodyLoaded { message });
    }
    state.backfill.finished(message, outcome);
    true
}

async fn serve(job: Job, parts: &EngineParts, pool: &Pool, state: &mut State) {
    match job {
        Job::Drain { reply } => {
            let outcome = drain(parts, pool, state).await;
            announce_drain(&parts.events, &outcome);
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
            let _ = reply.send(outcome);
        }
        Job::RequestBody { message, reply } => {
            let outcome = with_connection(pool, |connection| {
                backfill::request_body(connection, &mut state.backfill, message)
                    .map_err(|error| EngineError::new(error.to_string()))
            });
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
    }
}

/// One drain pass, with SMTP wired in so `Operation::Send` can actually send.
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

/// Why the link is not usable, phrased for the user.
fn offline_reason(link: &Link) -> String {
    match link {
        Link::Blocked(blocker) => blocker.reason().to_string(),
        Link::Offline => "there is no network".to_string(),
        Link::Waiting { .. } => "not connected yet".to_string(),
        Link::Online { .. } => "connected".to_string(),
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
    let status = state.status.on_link(&link);
    let connection = match &status {
        SyncStatus::Offline => postio_core::ConnectionState::Offline,
        SyncStatus::Connecting => postio_core::ConnectionState::Connecting,
        SyncStatus::Idle { .. } | SyncStatus::Syncing { .. } => {
            postio_core::ConnectionState::Online
        }
        SyncStatus::Error { .. } => postio_core::ConnectionState::Failing,
    };
    parts.events.emit(Event::ConnectionChanged {
        account: parts.account,
        state: connection,
    });
    // `ConnectionState::Failing` carries no reason of its own, deliberately.
    // The reason travels beside it, which is what the status line reads.
    if let SyncStatus::Error { reason, .. } = &status {
        parts.events.emit(Event::Error {
            message: reason.clone(),
        });
    }
}

/// Say what a drain did, so the UI hears it the way it hears everything else.
fn announce_drain(events: &EventSink, outcome: &Result<DrainSummary, EngineError>) {
    match outcome {
        Ok(summary) => {
            // A mailbox the server disagreed with has to be re-read, and the
            // list showing it is the thing that has to know.
            for mailbox in &summary.needs_resync {
                events.emit(Event::MessageListChanged { mailbox: *mailbox });
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
