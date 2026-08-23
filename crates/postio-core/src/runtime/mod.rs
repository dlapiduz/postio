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
//!
//! Both report back as [`Event`]s on the sink they were given, so the UI
//! learns what happened the same way it learns everything else.

use std::fmt;
use std::sync::Arc;

use chrono::Utc;
use postio_imap::backend::MailBackend;
use postio_imap::secret::SecretStore;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_smtp::transport::SmtpConnector;
use postio_storage::{BlobStore, Database, Pool};
use postio_sync::{Backfill, BackfillPolicy, Drainer, RetryPolicy, SmtpContext, backfill};

use crate::Event;
use crate::bridge::EventSink;

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
}

/// One unit of work for the engine's thread.
enum Job {
    Drain {
        account: AccountId,
        reply: tokio::sync::oneshot::Sender<Result<DrainSummary, EngineError>>,
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
            Job::Drain { account, .. } => write!(formatter, "Drain({account})"),
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
    pub async fn drain(&self, account: AccountId) -> Result<DrainSummary, EngineError> {
        self.ask(|reply| Job::Drain { account, reply }).await
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
        let mut backfill = Backfill::new(parts.backfill);
        while let Ok(job) = inbox.recv().await {
            match job {
                Job::Drain { account, reply } => {
                    let outcome = drain(&parts, &pool, account).await;
                    announce(&parts.events, &outcome);
                    let _ = reply.send(outcome);
                }
                Job::SeedBackfill {
                    mailbox,
                    limit,
                    reply,
                } => {
                    let outcome = with_connection(&pool, |connection| {
                        backfill::seed(connection, &mut backfill, mailbox, limit)
                            .map_err(|error| EngineError::new(error.to_string()))
                    });
                    let _ = reply.send(outcome);
                }
                Job::RequestBody { message, reply } => {
                    let outcome = with_connection(&pool, |connection| {
                        backfill::request_body(connection, &mut backfill, message)
                            .map_err(|error| EngineError::new(error.to_string()))
                    });
                    let _ = reply.send(outcome);
                }
            }
        }
    });
}

/// One drain pass, with SMTP wired in so `Operation::Send` can actually send.
async fn drain(
    parts: &EngineParts,
    pool: &Pool,
    account: AccountId,
) -> Result<DrainSummary, EngineError> {
    // A drain needs a session. Without one the queue is not *failed*, it is
    // simply not sent yet — which is the whole local-first promise: the write
    // already happened here, and reaching the server is a separate thing that
    // can wait. So a connection that will not open leaves every row where it
    // is and is reported as a connection problem, not as an operation that
    // went wrong.
    connect(parts, account).await?;

    let connection = pool
        .get()
        .map_err(|error| EngineError::new(error.to_string()))?;

    let smtp = SmtpContext {
        connector: parts.smtp.as_ref(),
        secrets: parts.secrets.as_ref(),
        blobs: &parts.blobs,
    };
    let drainer = Drainer::with_policy(parts.backend.as_ref(), parts.retry).with_smtp(smtp);

    let report = drainer
        .drain(&connection, account, Utc::now())
        .await
        .map_err(|error| EngineError::new(error.to_string()))?;

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

/// Make sure there is a session to drain over, opening one if there is not.
///
/// `capabilities` is the cheap question — it answers from the session that is
/// already open — so the common case costs nothing. Only when there is no
/// session does this dial.
async fn connect(parts: &EngineParts, account: AccountId) -> Result<(), EngineError> {
    if parts.backend.capabilities().await.is_ok() {
        return Ok(());
    }
    match parts.backend.connect().await {
        Ok(_) => {
            parts.events.emit(Event::ConnectionChanged {
                account,
                state: crate::ConnectionState::Online,
            });
            Ok(())
        }
        Err(error) => {
            parts.events.emit(Event::ConnectionChanged {
                account,
                state: crate::ConnectionState::Failing,
            });
            Err(EngineError::new(error.to_string()))
        }
    }
}

/// Say what a drain did, so the UI hears it the way it hears everything else.
fn announce(events: &EventSink, outcome: &Result<DrainSummary, EngineError>) {
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
