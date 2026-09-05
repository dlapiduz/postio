//! One operation per key at a time, and one result for everybody waiting.
//!
//! # Why this exists
//!
//! ADR 0006 Q5. With an engine per account, an OAuth provider's access tokens
//! expire *together* — the pool's three sessions, the SMTP path and whatever
//! the user just clicked all discover it within the same second. Refreshing
//! per caller turns one expiry into N refreshes against the token endpoint,
//! and on a provider that rotates its refresh token on every use, N-1 of them
//! present a token the server has already burned. The stampede is the normal
//! case here, not the edge.
//!
//! # Why the result is shared rather than the work merely serialised
//!
//! A mutex around the refresh would be simpler and would be wrong. Each
//! waiter would take it in turn, find the cache still empty because the
//! refresh *failed*, and try again — so a revoked grant would produce exactly
//! the storm this is meant to prevent, aimed at an endpoint that is already
//! saying no. Failures are shared for the same reason successes are.
//!
//! # Cancellation
//!
//! A leader whose future is dropped mid-flight — the caller went away —
//! leaves the registry through a guard, so the *next* caller leads rather
//! than joining a flight nobody is flying. Waiters already parked on it see
//! the channel close and fall back to doing the work themselves, which is the
//! honest answer: the alternative is reporting a failure nobody observed.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Mutex;

use tokio::sync::watch;

/// A registry of in-flight operations, one per key.
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

/// Which side of a flight this caller is on.
enum Role<T> {
    /// Nobody was running it, so this caller does — and publishes.
    Lead(watch::Sender<Option<T>>),
    /// Somebody already is. Wait for what they get.
    Follow(watch::Receiver<Option<T>>),
}

/// Leaves the registry however the leader's future ends, cancellation
/// included.
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
    /// Runs `work` for `key`, or waits for the run already under way and
    /// answers with its result.
    ///
    /// Every caller gets the same value, whether that value is a success or a
    /// failure. Two calls that do not overlap both do the work: this
    /// coalesces concurrency, and is not a cache.
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

    /// Work that counts how many times it actually ran and yields in the
    /// middle, so callers started together really do overlap.
    async fn counted(runs: Arc<AtomicUsize>, answer: &str) -> String {
        runs.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        answer.to_owned()
    }

    /// `n` callers asking for `key` at once, through tasks so they are really
    /// concurrent rather than merely interleaved by one `join`.
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
        // The stampede ADR 0006 Q5 describes: a pool's sessions and the SMTP
        // path all discover the same expiry within the same second.
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
        // The case a plain mutex gets wrong. Serialising the work would let
        // each waiter take its turn, find that the refresh had failed, and
        // try again -- pointing exactly the storm this prevents at an
        // endpoint that is already saying no.
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
        // This coalesces concurrency; it is not a cache, and a type that
        // quietly became one would hand out a token long after it expired.
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
        // A cancelled caller -- the user closed the window mid-refresh. The
        // registry entry has to go with it. A caller that merely falls back
        // to doing the work when it finds a dead flight looks fine on its
        // own and is not: two of them would each do the work, and the
        // account would have lost single-flight for the life of the process.
        // So the assertion is that a *new flight forms*, not that an answer
        // comes back.
        let flight = Arc::new(Registry::default());
        let runs = Arc::new(AtomicUsize::new(0));

        // The work sleeps for 20ms, so 5ms is long enough to claim the flight
        // and far too short to finish it. Dropping the timed-out future is
        // what a cancelled caller does.
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
