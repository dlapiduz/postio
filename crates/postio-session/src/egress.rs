//! Persisting the egress log (#151). `record` must return immediately, so events go over a channel
//! to a thread that writes batched transactions behind a Background write-gate permit.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use postio_model::egress::{EgressEvent, EgressSink};
use postio_model::ids::AccountId;
use postio_storage::repository::EgressLogRepository;
use postio_storage::{Store, WritePriority};

/// How long `shutdown` waits for the writer thread to close its connection; bounded so a write-gate
/// wait cannot hold a quit open.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub struct EgressRecorder {
    sender: Mutex<Option<mpsc::Sender<EgressEvent>>>,
    /// The writer thread, joined on shutdown and drop: closing the last SQLCipher connection calls
    /// into libcrypto, and an unjoined thread could still be in it when exit handlers tore
    /// libcrypto down, a coredump (#699).
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl EgressRecorder {
    pub fn start(database: Store) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel::<EgressEvent>();
        let spawned = std::thread::Builder::new()
            .name("postio-egress".to_string())
            .spawn(move || {
                // A runtime of its own: the store is async. Multi-threaded because a
                // `current_thread` runtime refuses `block_in_place`, and a synchronous store read
                // from inside one aborts.
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "the egress log has no writer: {error}");
                        return;
                    }
                };
                while let Ok(first) = receiver.recv() {
                    // One commit per burst, not per socket.
                    let mut batch = vec![first];
                    while let Ok(event) = receiver.try_recv() {
                        batch.push(event);
                    }
                    runtime.block_on(async {
                        let Ok(connection) = database.connect().await else {
                            return;
                        };
                        let _permit = connection
                            .write_gate()
                            .acquire(WritePriority::Background)
                            .await;
                        if connection.execute("BEGIN IMMEDIATE", ()).await.is_err() {
                            return;
                        }
                        let log = EgressLogRepository::new(&connection);
                        for event in &batch {
                            if let Err(error) = log.record(event).await {
                                tracing::warn!(%error, "an egress event was not recorded");
                            }
                        }
                        if let Err(error) = connection.execute("COMMIT", ()).await {
                            tracing::warn!(%error, "an egress batch did not commit");
                        }
                    });
                }
            });
        let thread = match spawned {
            Ok(handle) => Some(handle),
            Err(error) => {
                // The sink still swallows events; failing to audit must not cost the user their
                // sync.
                tracing::error!(%error, "the egress writer thread did not start: {error}");
                None
            }
        };
        Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            thread: Mutex::new(thread),
        })
    }

    pub fn for_account(self: &Arc<Self>, account: AccountId) -> Arc<dyn EgressSink> {
        Arc::new(AccountEgress {
            account,
            inner: Arc::clone(self),
        })
    }

    /// Stops the writer thread and waits up to `SHUTDOWN_GRACE` for it to close its connection.
    /// Idempotent; a `record` after it is swallowed.
    pub fn shutdown(&self) {
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
            // Detached on purpose: a stalled writer must not hold a quit or a test open.
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
        // A closed channel means the writer is gone; the connection still happens, unaudited.
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

    #[tokio::test]
    async fn recorded_events_reach_the_store_with_the_account_stamped() {
        let database = test_support::memory().await;
        let recorder = EgressRecorder::start(database.clone());
        let connection = database.connect().await.expect("checkout");
        let account = test_support::account(&connection).await.id;
        drop(connection);

        recorder
            .for_account(account)
            .record(event("imap.example.com"));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let rows = loop {
            let connection = database.connect().await.expect("checkout");
            let rows = EgressLogRepository::new(&connection)
                .recent(10)
                .await
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

    #[tokio::test]
    async fn shutdown_joins_the_writer_thread_before_returning() {
        let database = test_support::memory().await;
        let recorder = EgressRecorder::start(database);
        recorder.record(event("imap.example.com"));
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

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let database = test_support::memory().await;
        let recorder = EgressRecorder::start(database);
        recorder.shutdown();
        recorder.shutdown();
    }
}
