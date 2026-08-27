//! Persisting the egress log (#151): the sink connectors report to, backed
//! by the store.
//!
//! Connectors call [`EgressSink::record`] from async transports and a
//! blocking discovery probe, so the sink must return immediately. This one
//! hands the event to a channel; a dedicated thread drains it into
//! `egress_log` in batched transactions behind a Background write-gate
//! permit, so recording a connection never contends with the keystroke
//! whose sync opened it.

use std::sync::Arc;
use std::sync::mpsc;

use postio_model::egress::{EgressEvent, EgressSink};
use postio_model::ids::AccountId;
use postio_storage::repository::EgressLogRepository;
use postio_storage::{Database, WritePriority};

/// The process-wide recorder: one writer thread, however many connectors.
pub struct EgressRecorder {
    sender: mpsc::Sender<EgressEvent>,
}

impl EgressRecorder {
    /// Start the writer thread over `database` and hand back the recorder.
    ///
    /// The thread ends when the last clone of the recorder is dropped — the
    /// channel closes and `recv` returns its error. Nothing waits on it at
    /// shutdown: an egress row racing process exit is a row about a
    /// connection that was itself racing process exit.
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
        if let Err(error) = spawned {
            // The sink still swallows events; the log just stays empty —
            // failing to audit must not cost the user their sync.
            tracing::error!(%error, "the egress writer thread did not start");
        }
        Arc::new(Self { sender })
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
}

impl EgressSink for EgressRecorder {
    fn record(&self, event: EgressEvent) {
        // A closed channel means the writer thread is gone; the connection
        // still happens, it is just not audited — see `start`.
        let _ = self.sender.send(event);
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
}
