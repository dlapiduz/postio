//! What a search produced: the hits, the count, and how long it took.
//!
//! Kept in this crate rather than alongside `postio-index`'s executor that
//! produces them, because these types have two consumers with very different
//! rights. The executor needs `rusqlite` to build them; the frontend consumes
//! them and must never link SQLite at all
//! (`scripts/checks/check-crate-boundaries.py`). Keeping the shapes here lets
//! `postio-gtk` name the thing it is drawing instead of maintaining a
//! parallel copy of it that could drift.

use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_model::{EmailAddress, MailboxId, MessageId, ThreadId};

/// Which order a result set comes back in.
///
/// `Relevance` is the executor's ranked default — `bm25` folded with recency
/// and sender affinity. `Newest` is plain date order, the same order every
/// mailbox is in: what the list column's sort control switches to when the
/// ranking is not what the reader wants (#499). It lives here rather than in
/// `postio-index` because the frontend draws the control and must never link
/// SQLite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResultOrder {
    /// Ranked: the best answer first.
    #[default]
    Relevance,
    /// Date order, newest first — a mailbox's own order.
    Newest,
}

impl ResultOrder {
    /// The other one; what the sort control's toggle does.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Relevance => Self::Newest,
            Self::Newest => Self::Relevance,
        }
    }

    /// The control's label for this order.
    pub fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Relevance",
            Self::Newest => "Newest",
        }
    }
}

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
    /// Whether every message in the searched scope has its body indexed.
    ///
    /// `false` means the hits are drawn from a corpus that is still filling:
    /// headers sync long before bodies, so a message whose body has not
    /// arrived cannot match on anything it says. The count is then a floor
    /// for the same reason [`total_hits_capped`] makes it one, and the
    /// surface has to say so — a result set that reads as "this is what your
    /// mailbox contains" when it is not is the quiet kind of wrong (#352).
    ///
    /// Transient by design. ADR 0016 backfills every folder to completion by
    /// default, so this becomes `true` on its own and the caveat goes away —
    /// which is why the surface says *still syncing* rather than anything
    /// that reads as a permanent limitation.
    ///
    /// [`total_hits_capped`]: Self::total_hits_capped
    pub corpus_complete: bool,
}

/// The most `total_hits` will ever count exactly. See
/// [`SearchResults::total_hits_capped`].
pub const TOTAL_HITS_CAP: u64 = 10_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_relevance_gives_newest_and_back_again() {
        assert_eq!(ResultOrder::Relevance.toggled(), ResultOrder::Newest);
        assert_eq!(ResultOrder::Newest.toggled(), ResultOrder::Relevance);
    }

    #[test]
    fn toggling_twice_is_a_no_op() {
        for order in [ResultOrder::Relevance, ResultOrder::Newest] {
            assert_eq!(order.toggled().toggled(), order);
        }
    }

    #[test]
    fn each_order_names_itself_for_the_control() {
        assert_eq!(ResultOrder::Relevance.label(), "Relevance");
        assert_eq!(ResultOrder::Newest.label(), "Newest");
    }
}
