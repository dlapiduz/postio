//! Opt-in correlation of one invocation with the events it caused, for programmatic callers.
//! The untracked path (`CommandSender::send`) is unchanged and costs nothing.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::Event;

/// Process-wide so an id is unambiguous across runtimes in a test. Starts at 1: zero reads like an
/// absent value in a log.
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvocationId(u64);

impl InvocationId {
    /// For the extension path, which dispatches with a sink of its own rather than through the
    /// command queue.
    #[allow(clippy::should_implement_trait)]
    pub fn next() -> Self {
        InvocationId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for InvocationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Every invocation gets exactly one, including the ones that end badly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOutcome {
    Completed,
    Rejected {
        reason: String,
    },
    Failed {
        /// The failure, phrased for the user. Never contains a secret.
        message: String,
    },
}

/// `EventStream::next` unwraps it; the `_tracked` accessors hand it over intact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: Event,
    /// The tracked send that caused it, if any. `None` for everything the
    /// engine does on its own and for every fire-and-forget command.
    pub origin: Option<InvocationId>,
}

impl EventEnvelope {
    pub fn untracked(event: Event) -> Self {
        EventEnvelope {
            event,
            origin: None,
        }
    }

    pub fn is_from(&self, invocation: InvocationId) -> bool {
        self.origin == Some(invocation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_is_new() {
        let (first, second) = (InvocationId::next(), InvocationId::next());
        assert_ne!(first, second);
    }

    #[test]
    fn an_id_reads_as_an_id_in_a_log_line() {
        assert_eq!(InvocationId(42).to_string(), "#42");
    }

    #[test]
    fn an_event_belongs_to_at_most_one_invocation() {
        let mine = InvocationId::next();
        let theirs = InvocationId::next();
        let envelope = EventEnvelope {
            event: Event::Error {
                message: "no".into(),
                account: None,
            },
            origin: Some(mine),
        };
        assert!(envelope.is_from(mine));
        assert!(!envelope.is_from(theirs));
        assert!(!EventEnvelope::untracked(envelope.event).is_from(mine));
    }
}
