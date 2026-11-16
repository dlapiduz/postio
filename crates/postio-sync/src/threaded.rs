//! Which threads gained which messages in a pass (spec 013 T016).
//!
//! A pass threads every message it writes, then the engine says so once its
//! transaction has committed: `Event::ThreadChanged { thread, members_added }`.
//! One event per thread however many of its messages arrived together, so a
//! hundred replies to one thread are one event, not a hundred.

use std::collections::BTreeMap;

use postio_model::{MessageId, ThreadId};

/// The messages that joined each thread during a pass, in arrival order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadArrivals(BTreeMap<ThreadId, Vec<MessageId>>);

impl ThreadArrivals {
    /// `message` joined `thread`.
    pub fn add(&mut self, thread: ThreadId, message: MessageId) {
        self.0.entry(thread).or_default().push(message);
    }

    /// Fold `other` in, thread by thread.
    pub fn extend(&mut self, other: ThreadArrivals) {
        for (thread, messages) in other.0 {
            self.0.entry(thread).or_default().extend(messages);
        }
    }

    /// Whether nothing joined any thread.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Each thread and what joined it, by thread id.
    pub fn into_threads(self) -> impl Iterator<Item = (ThreadId, Vec<MessageId>)> {
        self.0.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrivals_to_one_thread_are_one_entry() {
        let mut arrivals = ThreadArrivals::default();
        arrivals.add(ThreadId::new(1), MessageId::new(10));
        arrivals.add(ThreadId::new(2), MessageId::new(20));
        arrivals.add(ThreadId::new(1), MessageId::new(11));
        let mut more = ThreadArrivals::default();
        more.add(ThreadId::new(1), MessageId::new(12));
        arrivals.extend(more);

        let threads: Vec<_> = arrivals.into_threads().collect();
        assert_eq!(
            threads,
            vec![
                (
                    ThreadId::new(1),
                    vec![MessageId::new(10), MessageId::new(11), MessageId::new(12)]
                ),
                (ThreadId::new(2), vec![MessageId::new(20)]),
            ]
        );
    }
}
