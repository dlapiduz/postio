//! Persisting the egress log (#151): the sink connectors report to, backed
//! by the store.
//!
//! Connectors call [`EgressSink::record`] from async transports and a
//! blocking discovery probe, so the sink must return immediately. This one
//! hands the event to a channel; a dedicated thread drains it into
//! `egress_log` in batched transactions behind a Background write-gate
//! permit, so recording a connection never contends with the keystroke
//! whose sync opened it.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use postio_model::egress::{EgressEvent, EgressSink};
use postio_model::ids::AccountId;
use postio_storage::repository::EgressLogRepository;
use postio_storage::{Database, WritePriority};

/// How long [`EgressRecorder::shutdown`] waits for the writer thread to
/// close its database connection, same discipline as
/// `postio_runtime::Engine`'s `SHUTDOWN_GRACE` and `postio_core::Bridge`'s
/// `DEFAULT_SHUTDOWN_TIMEOUT`: bounded, so a write-gate wait mid-flush cannot
/// hold a quit or a test open indefinitely.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// The process-wide recorder: one writer thread, however many connectors.
pub struct EgressRecorder {
    sender: Mutex<Option<mpsc::Sender<EgressEvent>>>,
    /// The writer thread, so [`shutdown`](Self::shutdown) — and this type's
    /// own [`Drop`] — can join it rather than leaving it to finish on its
    /// own.
    ///
    /// # The crash this exists for
    ///
    /// The closure `start` spawns owns the `Database` it was given, and
    /// closing the last connection to a SQLCipher database calls into
    /// libcrypto (`sqlite3FreeCodecArg`). Left to finish whenever it got
    /// around to it — the previous design here — that close could still be
    /// in flight when `main` returned and the process's exit handlers tore
    /// libcrypto down underneath it: a coredump inside a thread that was, on
    /// paper, "just an audit log write that does not matter if it is lost"
    /// (#699). Joining here is what makes that impossible: nothing can call
    /// `exit()` while this handle is still held.
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl EgressRecorder {
    /// Start the writer thread over `database` and hand back the recorder.
    pub fn start(database: Database) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel::<EgressEvent>();
        let spawned = std::thread::Builder::new()
            .name("postio-egress".to_string())
            .spawn(move || {
                while let Ok(first) = receiver.recv() {
                    // Whatever else queued while we slept goes in the same
                    // transaction: one commit per burst, not per socket.
                    let mut batch = vec![first];
                    while let Ok(event) = receiver.try_recv() {
                        batch.push(event);
                    }
                    let Ok(connection) = database.connection() else {
                        continue;
                    };
                    let _permit = connection.write_gate().acquire(WritePriority::Background);
                    if connection.execute_batch("BEGIN IMMEDIATE").is_err() {
                        continue;
                    }
                    let log = EgressLogRepository::new(&connection);
                    for event in &batch {
                        if let Err(error) = log.record(event) {
                            tracing::warn!(%error, "an egress event was not recorded");
                        }
                    }
                    if let Err(error) = connection.execute_batch("COMMIT") {
                        tracing::warn!(%error, "an egress batch did not commit");
                    }
                }
            });
        let thread = match spawned {
            Ok(handle) => Some(handle),
            Err(error) => {
                // The sink still swallows events; the log just stays empty —
                // failing to audit must not cost the user their sync.
                tracing::error!(%error, "the egress writer thread did not start");
                None
            }
        };
        Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            thread: Mutex::new(thread),
        })
    }

    /// This recorder as the sink for one account's connectors: every event
    /// it forwards carries `account`, which the transports themselves do
    /// not know.
    pub fn for_account(self: &Arc<Self>, account: AccountId) -> Arc<dyn EgressSink> {
        Arc::new(AccountEgress {
            account,
            inner: Arc::clone(self),
        })
    }

    /// Stops the writer thread and waits, up to [`SHUTDOWN_GRACE`], for it
    /// to close its database connection — see [`thread`](Self::thread) for
    /// why that wait matters.
    ///
    /// Idempotent, and safe to call while other clones of this recorder are
    /// still held elsewhere: a [`record`](EgressSink::record) after this
    /// point is swallowed exactly like one that raced a closed channel
    /// always was.
    pub fn shutdown(&self) {
        // Dropping the sender is what ends the writer thread's `recv` loop.
        drop(lock(&self.sender).take());

        let Some(handle) = lock(&self.thread).take() else {
            return;
        };
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            // Detached, deliberately: a stalled writer must not hold a quit
            // or a test open. This is the state everything was in before
            // the handle was kept, so the risk is the one that was always
            // there.
            tracing::warn!(
                "the egress writer thread did not stop within {SHUTDOWN_GRACE:?}; leaving it running"
            );
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl EgressSink for EgressRecorder {
    fn record(&self, event: EgressEvent) {
        // A closed channel means the writer thread is gone; the connection
        // still happens, it is just not audited — see `shutdown`.
        if let Some(sender) = lock(&self.sender).as_ref() {
            let _ = sender.send(event);
        }
    }
}

impl Drop for EgressRecorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// [`EgressRecorder`], stamped with the account its connectors serve.
struct AccountEgress {
    account: AccountId,
    inner: Arc<EgressRecorder>,
}

impl EgressSink for AccountEgress {
    fn record(&self, mut event: EgressEvent) {
        event.account = Some(self.account);
        self.inner.record(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use postio_model::egress::{EgressOutcome, EgressSubsystem};
    use postio_storage::test_support;

    fn event(host: &str) -> EgressEvent {
        EgressEvent {
            at: Utc::now(),
            subsystem: EgressSubsystem::Imap,
            account: None,
            host: host.to_string(),
            port: 993,
            outcome: EgressOutcome::Connected,
        }
    }

    #[test]
    fn recorded_events_reach_the_store_with_the_account_stamped() {
        let database = test_support::memory();
        let recorder = EgressRecorder::start(database.clone());
        let connection = database.connection().expect("checkout");
        let account = test_support::account(&connection).id;
        drop(connection);

        recorder
            .for_account(account)
            .record(event("imap.example.com"));

        // The writer is a thread; give it a moment rather than a hook —
        // the deadline is generous and the pass is immediate in practice.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let rows = loop {
            let connection = database.connection().expect("checkout");
            let rows = EgressLogRepository::new(&connection)
                .recent(10)
                .expect("recent");
            if !rows.is_empty() || std::time::Instant::now() > deadline {
                break rows;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        assert_eq!(rows.len(), 1, "the event crossed the channel and the store");
        assert_eq!(rows[0].host, "imap.example.com");
        assert_eq!(
            rows[0].account,
            Some(account),
            "the per-account sink stamps what the transport cannot know"
        );
    }

    #[test]
    fn shutdown_joins_the_writer_thread_before_returning() {
        let database = test_support::memory();
        let recorder = EgressRecorder::start(database);
        recorder.record(event("imap.example.com"));
        // Give the writer thread a moment to reach its main loop, so
        // shutdown has something running to actually join rather than a
        // thread that has not started yet.
        std::thread::sleep(std::time::Duration::from_millis(20));

        recorder.shutdown();

        assert!(
            recorder.thread.lock().unwrap().is_none(),
            "shutdown must join the writer thread, not merely close the \
             channel and move on -- a thread still closing its SQLCipher \
             connection when the process exits is #699 (SIGSEGV in \
             sqlite3FreeCodecArg, racing libcrypto's own teardown)"
        );
    }

    #[test]
    fn shutdown_is_idempotent() {
        let database = test_support::memory();
        let recorder = EgressRecorder::start(database);
        recorder.shutdown();
        recorder.shutdown();
    }
}
