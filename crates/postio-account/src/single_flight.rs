//! ADR 0006 Q5: an OAuth provider's tokens expire together, so refreshing per caller would stampede
//! the endpoint and burn rotating refresh tokens. The result is shared, failures included: a mutex
//! would let each waiter retry a revoked grant. A dropped leader leaves the registry through a
//! guard, and parked waiters fall back to doing the work themselves.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Mutex;

use tokio::sync::watch;

#[derive(Debug)]
pub(crate) struct SingleFlight<K, T> {
    inflight: Mutex<HashMap<K, watch::Receiver<Option<T>>>>,
}

impl<K, T> Default for SingleFlight<K, T> {
    fn default() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }
}

enum Role<T> {
    Lead(watch::Sender<Option<T>>),
    Follow(watch::Receiver<Option<T>>),
}

struct Flight<'a, K: Clone + Eq + Hash, T> {
    registry: &'a Mutex<HashMap<K, watch::Receiver<Option<T>>>>,
    key: K,
}

impl<K: Clone + Eq + Hash, T> Drop for Flight<'_, K, T> {
    fn drop(&mut self) {
        self.registry
            .lock()
            .expect("single-flight registry mutex")
            .remove(&self.key);
    }
}

impl<K: Clone + Eq + Hash, T: Clone> SingleFlight<K, T> {
    /// Coalesces concurrency; it is not a cache: calls that do not overlap both do the work.
    pub(crate) async fn run<F>(&self, key: &K, work: F) -> T
    where
        F: Future<Output = T>,
    {
        // The look-up and the claim are one critical section, or two callers
        // arriving together would both find nothing and both lead.
        let role = {
            let mut inflight = self.inflight.lock().expect("single-flight registry mutex");
            match inflight.get(key) {
                Some(receiver) => Role::Follow(receiver.clone()),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    inflight.insert(key.clone(), receiver);
                    Role::Lead(sender)
                }
            }
        };

        let mut receiver = match role {
            Role::Lead(sender) => return self.lead(key, sender, work).await,
            Role::Follow(receiver) => receiver,
        };
        loop {
            let published = receiver.borrow_and_update().clone();
            if let Some(outcome) = published {
                return outcome;
            }
            if receiver.changed().await.is_err() {
                // The leader went away without answering. Nobody has a result
                // to share, so this caller becomes one that does the work.
                return work.await;
            }
        }
    }

    async fn lead<F>(&self, key: &K, sender: watch::Sender<Option<T>>, work: F) -> T
    where
        F: Future<Output = T>,
    {
        let flight = Flight {
            registry: &self.inflight,
            key: key.clone(),
        };
        let outcome = work.await;
        // Out of the registry *before* the result is published, so a caller
        // arriving from here on starts a fresh flight rather than joining one
        // that is already over.
        drop(flight);
        let _ = sender.send(Some(outcome.clone()));
        outcome
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    type Registry = SingleFlight<String, String>;

    fn account() -> String {
        "ada@example.com".to_owned()
    }

    async fn counted(runs: Arc<AtomicUsize>, answer: &str) -> String {
        runs.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        answer.to_owned()
    }

    async fn all_at_once(
        flight: &Arc<Registry>,
        runs: &Arc<AtomicUsize>,
        key: &str,
        n: usize,
    ) -> Vec<String> {
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..n {
            let (flight, runs, key) = (Arc::clone(flight), Arc::clone(runs), key.to_owned());
            tasks.spawn(async move {
                flight
                    .run(&key, counted(Arc::clone(&runs), "one-token"))
                    .await
            });
        }
        let mut answers = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            answers.push(joined.expect("the task should not panic"));
        }
        answers
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn callers_that_overlap_share_one_run() {
        let flight = Arc::new(Registry::default());
        let runs = Arc::new(AtomicUsize::new(0));

        let answers = all_at_once(&flight, &runs, &account(), 8).await;

        assert_eq!(runs.load(Ordering::SeqCst), 1, "one refresh, not eight");
        assert_eq!(answers.len(), 8);
        assert!(
            answers.iter().all(|answer| answer == "one-token"),
            "and every caller got it: {answers:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_failure_is_shared_rather_than_retried_by_every_waiter() {
        let flight = Arc::new(SingleFlight::<String, Result<String, String>>::default());
        let runs = Arc::new(AtomicUsize::new(0));

        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..5 {
            let (flight, runs) = (Arc::clone(&flight), Arc::clone(&runs));
            tasks.spawn(async move {
                flight
                    .run(&account(), async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        Err::<String, String>("the grant was revoked".to_owned())
                    })
                    .await
            });
        }
        let mut answers = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            answers.push(joined.expect("the task should not panic"));
        }

        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(answers.len(), 5);
        assert!(answers.iter().all(Result::is_err));
    }

    #[tokio::test]
    async fn two_calls_that_do_not_overlap_both_do_the_work() {
        let flight = Registry::default();
        let runs = Arc::new(AtomicUsize::new(0));

        flight
            .run(&account(), counted(Arc::clone(&runs), "a"))
            .await;
        flight
            .run(&account(), counted(Arc::clone(&runs), "b"))
            .await;

        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_account_refreshing_does_not_hold_up_another() {
        let flight = Arc::new(Registry::default());
        let runs = Arc::new(AtomicUsize::new(0));

        let first = all_at_once(&flight, &runs, "ada@example.com", 2);
        let second = all_at_once(&flight, &runs, "quinn@example.net", 2);
        let (first, second) = tokio::join!(first, second);

        assert_eq!(runs.load(Ordering::SeqCst), 2, "per account, not global");
        assert_eq!(first.len() + second.len(), 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_leader_that_goes_away_leaves_the_next_flight_free_to_form() {
        // A cancelled caller must take its registry entry with it; assert that a new flight forms,
        // not just that an answer returns, since a fallback would hide the lost single-flight.
        let flight = Arc::new(Registry::default());
        let runs = Arc::new(AtomicUsize::new(0));

        let abandoned = tokio::time::timeout(
            Duration::from_millis(5),
            flight.run(&account(), counted(Arc::clone(&runs), "never")),
        )
        .await;
        assert!(abandoned.is_err(), "the leader was cancelled mid-flight");
        assert_eq!(runs.load(Ordering::SeqCst), 1, "it did start");

        let answers = all_at_once(&flight, &runs, &account(), 4).await;

        assert_eq!(answers.len(), 4);
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "the abandoned run, and one shared between the four that followed"
        );
    }
}
