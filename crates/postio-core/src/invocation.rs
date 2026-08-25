//! Correlating one invocation with the events it caused.
//!
//! The bus is fire-and-forget, and for GTK that is exactly right: a repaint
//! does not care which keystroke caused it, and every widget on screen wants
//! every event. A *programmatic* caller — MCP, AI, a future CLI — needs the
//! opposite. It sent one archive and has to know whether that archive
//! succeeded, on a stream where the sync engine emits
//! [`MessagesChanged`](crate::Event::MessagesChanged) constantly for reasons of
//! its own. Matching by shape and timing stops working the moment two commands
//! are in flight.
//!
//! So a caller may opt in:
//!
//! ```
//! use postio_core::bridge::Bridge;
//! use postio_core::dispatch::Dispatcher;
//! use postio_core::invocation::InvocationOutcome;
//! use postio_core::{Command, CommandId, Event};
//!
//! let dispatcher = Dispatcher::builder()
//!     .on(CommandId::Refresh, |invocation| async move {
//!         invocation.emit(Event::MailboxesChanged {
//!             account: postio_model::AccountId::new(1),
//!         });
//!         Ok(())
//!     })
//!     .build();
//!
//! let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
//! let mine = bridge.commands().send_tracked(Command::Refresh).expect("running");
//! bridge.shutdown();
//!
//! let ours: Vec<_> = std::iter::from_fn(|| events.try_next_tracked())
//!     .filter(|envelope| envelope.is_from(mine))
//!     .map(|envelope| envelope.event)
//!     .collect();
//! assert!(matches!(
//!     ours.last(),
//!     Some(Event::InvocationFinished { outcome: InvocationOutcome::Completed, .. })
//! ));
//! ```
//!
//! # What this deliberately does not do
//!
//! It does not change the untracked path at all. [`CommandSender::send`] still
//! carries no id, its events still arrive with `origin: None`, and it still
//! announces no completion — so the GTK frontend sees the same stream it saw
//! before, event for event. Tracking costs nothing when nobody asks for it.
//!
//! It also does not fan the stream out. That is
//! [`EventHub`](crate::bridge::EventHub)'s job, decided separately in
//! ADR 0013: every subscriber gets a private
//! [`EventStream`](crate::bridge::EventStream) and sees every envelope, so
//! "who sees a repaint, who sees a rejection" is answered "everyone", and
//! *which of those are mine* stays this module's question — [`is_from`]
//! filters per subscriber, because an [`InvocationId`] is process-unique
//! rather than stream-unique.
//!
//! [`is_from`]: EventEnvelope::is_from
//!
//! [`CommandSender::send`]: crate::bridge::CommandSender::send

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::Event;

/// Hands out invocation ids, process-wide.
///
/// Process-wide rather than per-[`Bridge`](crate::bridge::Bridge) so an id is
/// unambiguous even in a test that runs several runtimes, and so an id can be
/// written to a log without needing to say which bus it came from. It starts
/// at 1: zero reads like an absent value in a log line, and this type has
/// `None` for that.
static NEXT: AtomicU64 = AtomicU64::new(1);

/// One send of one command, told apart from every other.
///
/// Handed out by
/// [`CommandSender::send_tracked`](crate::bridge::CommandSender::send_tracked),
/// and carried by every event that send caused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvocationId(u64);

impl InvocationId {
    /// Take the next id.
    ///
    /// Callers who dispatch through
    /// [`CommandSender::send_tracked`](crate::bridge::CommandSender::send_tracked)
    /// get one back and never need this. It is here for the extension path,
    /// which dispatches through
    /// [`Dispatcher::dispatch_ext`](crate::dispatch::Dispatcher::dispatch_ext)
    /// with a sink of its own rather than through the command queue.
    #[allow(clippy::should_implement_trait)]
    pub fn next() -> Self {
        InvocationId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// The id as a number, for a log line or a wire format.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for InvocationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// How a tracked invocation ended.
///
/// Every invocation gets exactly one of these, including the ones that end
/// badly: a caller awaiting an answer that never comes is worse than a caller
/// told the handler panicked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOutcome {
    /// The handler ran and returned `Ok`.
    Completed,
    /// The command was not applicable — nothing selected, nothing to undo, no
    /// handler in this build. The prose already arrived as
    /// [`Event::CommandRejected`].
    Rejected {
        /// Why, phrased for the user.
        reason: String,
    },
    /// The command was applicable and went wrong, or its handler panicked.
    /// The prose already arrived as [`Event::Error`].
    Failed {
        /// The failure, phrased for the user. Never contains a secret.
        message: String,
    },
}

/// One event, and the invocation it answers.
///
/// What the channel actually carries.
/// [`EventStream::next`](crate::bridge::EventStream::next) unwraps it, because
/// the frontend has no use for the envelope; the `_tracked` accessors hand it
/// over intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnvelope {
    /// What happened.
    pub event: Event,
    /// The tracked send that caused it, if any. `None` for everything the
    /// engine does on its own and for every fire-and-forget command.
    pub origin: Option<InvocationId>,
}

impl EventEnvelope {
    /// An event nobody asked for: engine noise, or an untracked command.
    pub fn untracked(event: Event) -> Self {
        EventEnvelope {
            event,
            origin: None,
        }
    }

    /// Whether this event answers `invocation`.
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
            },
            origin: Some(mine),
        };
        assert!(envelope.is_from(mine));
        assert!(!envelope.is_from(theirs));
        assert!(!EventEnvelope::untracked(envelope.event).is_from(mine));
    }
}
