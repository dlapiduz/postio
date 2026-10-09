//! What a search produced: the hits, the count, and how long it took.
//!
//! Kept in this crate rather than alongside `postio-index`'s executor that
//! produces them, because these types have two consumers with very different
//! rights. The executor needs `rusqlite` to build them; the frontend consumes
//! them and must never link SQLite at all
//! (`scripts/checks/check-crate-boundaries.py`). Keeping the shapes here lets
//! a frontend name the thing it is drawing instead of maintaining a
//! parallel copy of it that could drift.

use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_model::{
    AddressId, AttachmentId, EmailAddress, LabelId, MailboxId, MessageId, ThreadId,
};

use crate::facets::SearchFacets;
pub use crate::passage::Passage;

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
    /// The message's first line, as the list shows it: what tells two hits
    /// with one subject apart without opening either.
    pub preview: Option<String>,
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
    /// A term to search for instead, when this query found nothing.
    ///
    /// Always `None` when there are hits: a query that worked is not one to
    /// second-guess. See ADR 0037 for why the tolerance is here, in what is
    /// *offered*, rather than in what the index matches.
    pub suggestion: Option<crate::suggest::Suggestion>,
    /// Set when these are the results for a different word than the one
    /// typed, because the typed one found nothing.
    ///
    /// Only the search box does this (`postio_session::search::execute`),
    /// never a saved search or a rule: those match what they say, which is
    /// ADR 0037. And the surface must say so, since the list no longer
    /// answers the letters in the box.
    pub instead: Option<Instead>,
}

/// The word a result set is for, in place of the one typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instead {
    /// What was in the box, and found nothing.
    pub typed: String,
    /// What the results are for.
    pub term: String,
}

/// The most `total_hits` will ever count exactly. See
/// [`SearchResults::total_hits_capped`].
///
/// **Ten thousand, and lowering it was tried and reverted.** The count is the
/// most expensive statement in a broad search -- measured at 152ms of a 241ms
/// search for `the` on a real store, against 74ms for the fetch -- and the
/// exact number buys little: nobody navigates by the difference between
/// "3,843 results" and "1,000+".
///
/// But the count is not only shown. `RANK_BY_RELEVANCE_LIMIT` (2,000) and
/// `PROBED_FORM_LIMIT` (8,000) both read it to choose a query plan, and a cap
/// below either silently pins every broad search to one side of those
/// thresholds: at 1,000 the count saturates before it can distinguish 3,843
/// matches from 10,000, so the probed shape is never chosen and `the` goes
/// from 74ms to 1.26s. Cheapening the display would have cost an order of
/// magnitude on exactly the queries it was meant to help.
///
/// Any future attempt has to separate the two readers first: a cheap
/// "how broad is this" signal for the planner, and a cap for the display.
/// FTS5 offers no cheap count, which is what makes that hard.
pub const TOTAL_HITS_CAP: u64 = 10_000;

// ---------------------------------------------------------------------------
// Conversation search (spec 010)
// ---------------------------------------------------------------------------

/// Where in a message a match was found: the row's source tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The subject line.
    Subject,
    /// The body, the person's own words.
    Body,
    /// The body's quoted history.
    Quoted,
    /// An attachment's file name.
    FileName {
        /// The attachment.
        attachment: AttachmentId,
        /// Its file name.
        name: String,
    },
    /// What an attachment says.
    FileContent {
        /// The attachment.
        attachment: AttachmentId,
        /// Its file name.
        name: String,
        /// Where in it.
        location: Location,
    },
}

/// Where in an attachment a passage sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A PDF page, 1-based.
    Page(u32),
    /// A spreadsheet row, 1-based, in a named sheet.
    Sheet {
        /// The sheet's name.
        name: String,
        /// The row, 1-based.
        row: u32,
    },
    /// A presentation slide, 1-based.
    Slide(u32),
    /// A document paragraph, 1-based.
    Paragraph(u32),
    /// A line of plain text, 1-based.
    Line(u32),
    /// Reserved: a row of a table in a document.
    Table {
        /// Which table, 1-based.
        index: u32,
        /// The row, 1-based.
        row: u32,
    },
    /// Reserved: text read from an image.
    ImageText,
}

/// One place a message matched, and the words around it once they are cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Where.
    pub source: Source,
    /// The words around it; `None` until `Req::Passages` fills it, or when
    /// there is no local text to cut it from.
    pub passage: Option<Passage>,
    /// When the matching message arrived.
    pub when: Option<DateTime<Utc>>,
}

/// One match in a conversation, as Quick Look lists them (spec 010 US4):
/// a [`Match`] with the message it is in and who wrote that message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMatch {
    /// The message it is in; `None` for the conversation's subject, which
    /// is one match however many messages carry it.
    pub message: Option<MessageId>,
    /// Who wrote the words; `None` for the subject and for quoted history,
    /// whose writer the quote does not say.
    pub from: Option<EmailAddress>,
    /// Where, the words around it, and when it was sent.
    pub found: Match,
}

/// Why a conversation ranks where it does: the row's reason line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankReason {
    /// The person replied in it.
    Replied,
    /// The person flagged it.
    Flagged,
    /// From someone the person often hears from.
    FrequentSender,
    /// The words are in the subject.
    InSubject,
    /// The words are in an attachment's name.
    InFileName,
    /// How many of its messages matched; always last.
    Matches(u32),
}

/// What one result row stands for: a thread, or a message threading never
/// joined to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationKey {
    /// A thread.
    Thread(ThreadId),
    /// A message on its own.
    Lone(MessageId),
}

/// One conversation that matched.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationHit {
    /// Which conversation.
    pub key: ConversationKey,
    /// The message the row shows and opens: its best match.
    pub best: MessageId,
    /// The best message's folder.
    pub mailbox_id: MailboxId,
    /// The best message's subject.
    pub subject: Option<String>,
    /// The best message's sender.
    pub from: Option<EmailAddress>,
    /// When its newest matching message arrived: decides its month (D4).
    pub newest_match: DateTime<Utc>,
    /// Messages in the conversation: the count badge.
    pub messages: u32,
    /// Anything in it unread.
    pub unread: bool,
    /// Anything in it with an attachment.
    pub has_attachments: bool,
    /// Its labels.
    pub labels: Vec<LabelId>,
    /// Lower is better, as [`SearchHit::score`].
    pub score: f64,
    /// Why it ranks here; `Matches` always last.
    pub reasons: Vec<RankReason>,
    /// Where it matched; passages filled by a read of their own.
    pub matches: Vec<Match>,
}

/// What one conversation search produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationResults {
    /// This page.
    pub hits: Vec<ConversationHit>,
    /// Conversations that match, up to the cap.
    pub total: u64,
    /// Whether `total` is a floor.
    pub capped: bool,
    /// Messages the search looked through: "Searched all 18,204 messages".
    pub messages_searched: u64,
    /// Every message's body is indexed (as [`SearchResults::corpus_complete`]).
    pub corpus_complete: bool,
    /// Every downloaded attachment's text is extracted.
    pub contents_complete: bool,
    /// The filter buttons' and timeline's counts.
    pub facets: SearchFacets,
    /// Matching files: the Files tab's count.
    pub files: u64,
    /// Matching people: the People tab's count.
    pub people: u64,
    /// The names behind the facets' ids and the hits' labels. Empty from
    /// the executor; `postio-session` reads them.
    pub names: FacetNames,
    /// How long it took.
    pub elapsed: Duration,
}

/// What the ids a conversation search returns are called: the filter
/// buttons' and popovers' words ("From: Ada Moreno"), and the rows' label
/// pills.
///
/// Read once per kind for the counts a search returns rather than carried
/// on each count, so a facet of fifty people costs one read, not fifty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacetNames {
    /// Senders and recipients: the address, with the name its mail gave it.
    pub people: Vec<(AddressId, EmailAddress)>,
    /// Labels, by their names.
    pub labels: Vec<(LabelId, String)>,
    /// The colours labels were given (`#rrggbb`), for the rows' pills;
    /// a label with none is not here.
    pub label_colors: Vec<(LabelId, String)>,
    /// Folders, by their names.
    pub folders: Vec<(MailboxId, String)>,
}

/// The results view's two orders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConversationOrder {
    /// Ranked, with Top hits first.
    #[default]
    BestMatch,
    /// Newest first, in month groups only.
    Newest,
}

impl From<ConversationOrder> for ResultOrder {
    fn from(order: ConversationOrder) -> Self {
        match order {
            ConversationOrder::BestMatch => ResultOrder::Relevance,
            ConversationOrder::Newest => ResultOrder::Newest,
        }
    }
}

/// The results view's tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResultsTab {
    /// Conversations.
    #[default]
    Conversations,
    /// Files.
    Files,
    /// People.
    People,
}

/// One attachment that matched: a card on the Files tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    /// The attachment.
    pub attachment: AttachmentId,
    /// The message carrying it.
    pub message: MessageId,
    /// Its file name.
    pub name: String,
    /// Its MIME type.
    pub mime_type: String,
    /// Its size in bytes.
    pub size: u64,
    /// Who sent it.
    pub from: Option<EmailAddress>,
    /// When it arrived.
    pub received_at: DateTime<Utc>,
    /// The message's subject.
    pub subject: Option<String>,
    /// Where it matched: its name, or a located passage of its content.
    pub matched: Option<Match>,
}

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
