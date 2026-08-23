//! What a search produced: the hits, the count, and how long it took.
//!
//! Separate from [`executor`](crate::executor), and deliberately *not* behind
//! the `index` cargo feature, because these types have two consumers with
//! very different rights. The executor produces them and needs `rusqlite` to
//! do it; the frontend consumes them and must never link SQLite at all
//! (`scripts/check-crate-boundaries.py`). Keeping the shapes here lets
//! `postio-gtk` name the thing it is drawing instead of maintaining a
//! parallel copy of it that could drift.

use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_model::{EmailAddress, MailboxId, MessageId, ThreadId};

/// One ranked, snippeted result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// The message.
    pub message_id: MessageId,
    /// Its thread, if threading has run.
    pub thread_id: Option<ThreadId>,
    /// Which mailbox holds this copy.
    pub mailbox_id: MailboxId,
    /// `Subject`, verbatim.
    pub subject: Option<String>,
    /// Who it is from.
    pub from: Option<EmailAddress>,
    /// When the server received it.
    pub received_at: DateTime<Utc>,
    /// A snippet of the matching text, with each match wrapped in the
    /// markers [`crate::highlight`] defines. Empty for a query with no free
    /// text to snippet; [`crate::highlight::from_snippet`] reads it back.
    pub snippet: String,
    /// The rank score: lower is a better match. Not meaningful on its own,
    /// only as an ordering.
    pub score: f64,
}

/// What one search produced, for the canvas 2b readout.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResults {
    /// This page of hits, best match first.
    pub hits: Vec<SearchHit>,
    /// The total number of messages that match, regardless of `limit`, up to
    /// [`TOTAL_HITS_CAP`]. See [`SearchResults::total_hits_capped`].
    pub total_hits: u64,
    /// Whether `total_hits` is a floor rather than the true count.
    ///
    /// A word common enough to sit in most of a large mailbox's messages
    /// still has to stay inside the `<100 ms` budget (CLAUDE.md), and an
    /// exact count of a match that broad means walking every one of
    /// them — there is no shortcut, because FTS5 does not expose a term's
    /// document frequency to plain SQL. So counting stops at
    /// [`TOTAL_HITS_CAP`]: when this is `true`, `total_hits` is exactly that
    /// cap and the readout should show "`{total_hits}+ hits`" rather than a
    /// number that reads as precise. Ordinary queries never reach the cap.
    pub total_hits_capped: bool,
    /// How long the search took, start to finish.
    pub elapsed: Duration,
}

/// The most `total_hits` will ever count exactly. See
/// [`SearchResults::total_hits_capped`].
pub const TOTAL_HITS_CAP: u64 = 10_000;
