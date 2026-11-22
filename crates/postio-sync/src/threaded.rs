//! Which threads gained which messages in a pass, so the engine emits one `ThreadChanged` per
//! thread (spec 013 T016).

use std::collections::BTreeMap;

use postio_model::{MessageId, ThreadId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadArrivals(BTreeMap<ThreadId, Vec<MessageId>>);

impl ThreadArrivals {
    pub fn add(&mut self, thread: ThreadId, message: MessageId) {
        self.0.entry(thread).or_default().push(message);
    }

    pub fn extend(&mut self, other: ThreadArrivals) {
        for (thread, messages) in other.0 {
            self.0.entry(thread).or_default().extend(messages);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

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
