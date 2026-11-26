//! What a search produced. Here rather than beside `postio-index`'s executor because a frontend
//! consumes these and must never link SQLite (`scripts/checks/check-crate-boundaries.py`).

use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_model::{
    AddressId, AttachmentId, EmailAddress, LabelId, MailboxId, MessageId, ThreadId,
};

use crate::facets::SearchFacets;
pub use crate::passage::Passage;

/// `Relevance` is the ranked default (`bm25` with recency and sender affinity); `Newest` is plain
/// date order (#499).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResultOrder {
    #[default]
    Relevance,
    Newest,
}

impl ResultOrder {
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Relevance => Self::Newest,
            Self::Newest => Self::Relevance,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Relevance",
            Self::Newest => "Newest",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub message_id: MessageId,
    pub thread_id: Option<ThreadId>,
    pub mailbox_id: MailboxId,
    pub subject: Option<String>,
    pub from: Option<EmailAddress>,
    pub received_at: DateTime<Utc>,
    pub preview: Option<String>,
    pub snippet: String,
    /// Lower is a better match; meaningful only as an ordering.
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total_hits: u64,
    /// Counting stops at [`TOTAL_HITS_CAP`] to stay in the 100 ms budget (FTS5 has no cheap count):
    /// when true, `total_hits` is exactly the cap and the readout shows "`{total_hits}+ hits`".
    pub total_hits_capped: bool,
    pub elapsed: Duration,
    /// `false` means bodies are still filling, so a message whose body has not arrived cannot match
    /// on it and the count is a floor; the surface must say *still syncing* (#352). Transient: ADR
    /// 0016 backfills every folder.
    pub corpus_complete: bool,
    /// Always `None` when there are hits. The tolerance is in what is offered, not in what the
    /// index matches (ADR 0037).
    pub suggestion: Option<crate::suggest::Suggestion>,
    /// Only the search box does this (`postio_session::search::execute`), never a saved search or a
    /// rule (ADR 0037); the surface must say so, since the list no longer answers the letters in
    /// the box.
    pub instead: Option<Instead>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instead {
    pub typed: String,
    pub term: String,
}

/// Ten thousand; lowering it was tried and reverted. The count is the costliest statement in a
/// broad search, but `RANK_BY_RELEVANCE_LIMIT` (2,000) and `PROBED_FORM_LIMIT` (8,000) read it to
/// choose a query plan, and a cap below either pins every broad search to one side: at 1,000 `the`
/// went from 74ms to 1.26s. Any future attempt has to separate a cheap "how broad" signal for the
/// planner from a display cap.
pub const TOTAL_HITS_CAP: u64 = 10_000;

/// Its own constant rather than [`TOTAL_HITS_CAP`] lowered, because the two bound different work:
/// the conversation walk reads every matched row whole and is linear in rows, while
/// `TOTAL_HITS_CAP` also feeds GTK's plan choice and the relaxations' count.
pub const CONVERSATION_WALK_CAP: u64 = 5_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counted {
    pub conversations: u64,
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Subject,
    Body,
    Quoted,
    FileName {
        attachment: AttachmentId,
        name: String,
    },
    FileContent {
        attachment: AttachmentId,
        name: String,
        location: Location,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Page(u32),
    Sheet { name: String, row: u32 },
    Slide(u32),
    Paragraph(u32),
    Line(u32),
    Table { index: u32, row: u32 },
    ImageText,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub source: Source,
    /// `None` until `Req::Passages` fills it, or when there is no local text to cut it from.
    pub passage: Option<Passage>,
    pub when: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMatch {
    /// `None` for the conversation's subject, which is one match however many messages carry it.
    pub message: Option<MessageId>,
    /// `None` for the subject and for quoted history, whose writer the quote does not say.
    pub from: Option<EmailAddress>,
    pub found: Match,
    pub to: Vec<Addressee>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addressee {
    pub address: EmailAddress,
    pub own: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankReason {
    Replied,
    Flagged,
    FrequentSender,
    InSubject,
    InSender,
    InFileName,
    InFileContent,
    Matches(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationKey {
    Thread(ThreadId),
    Lone(MessageId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationHit {
    pub key: ConversationKey,
    pub best: MessageId,
    pub best_number: u32,
    pub mailbox_id: MailboxId,
    pub subject: Option<String>,
    pub from: Option<EmailAddress>,
    pub newest_match: DateTime<Utc>,
    pub messages: u32,
    pub unread: bool,
    pub has_attachments: bool,
    pub labels: Vec<LabelId>,
    pub score: f64,
    pub reasons: Vec<RankReason>,
    pub matched_messages: u32,
    pub matches: Vec<Match>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationResults {
    pub hits: Vec<ConversationHit>,
    pub total: u64,
    pub capped: bool,
    pub messages_searched: u64,
    pub corpus_complete: bool,
    pub contents_complete: bool,
    pub facets: SearchFacets,
    pub files: u64,
    pub people: u64,
    pub names: FacetNames,
    pub elapsed: Duration,
}

/// Read once per kind rather than carried on each count, so a facet of fifty people costs one read,
/// not fifty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FacetNames {
    pub people: Vec<(AddressId, EmailAddress)>,
    /// The facets count them (you are in every message you sent) but nothing offers you as someone
    /// to narrow to.
    pub own: Vec<AddressId>,
    pub labels: Vec<(LabelId, String)>,
    pub label_colors: Vec<(LabelId, String)>,
    pub folders: Vec<(MailboxId, String)>,
    pub held: Vec<(MessageId, String, bool)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConversationOrder {
    #[default]
    BestMatch,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResultsTab {
    #[default]
    Conversations,
    Files,
    People,
    Events,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub attachment: AttachmentId,
    pub message: MessageId,
    pub name: String,
    pub mime_type: String,
    pub size: u64,
    pub from: Option<EmailAddress>,
    pub received_at: DateTime<Utc>,
    pub subject: Option<String>,
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
