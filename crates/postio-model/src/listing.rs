//! The rows and pages a frontend draws, and `MailStore`, the contract for reading them. Plain
//! serialisable data, so the store's one owner can answer a frontend in another process behind the
//! same `dyn MailStore` (ADR 0041).

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ListScope;
use crate::address::EmailAddress;
use crate::ids::{AccountId, MailboxId, MessageId, ThreadId};
use crate::mailbox::Mailbox;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub scope: ListScope,
    pub offset: u32,
    pub limit: u32,
}

/// Adds the thread count the row's badge shows and drops the size, which nothing draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: MessageId,
    pub thread: Option<ThreadId>,
    pub from: Option<EmailAddress>,
    pub subject: Option<String>,
    pub preview: Option<String>,
    /// What the row says, and the order of every list but a folder's, which a woken snooze tops.
    pub received_at: DateTime<Utc>,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    /// Carried to the row, not flattened to a bool: Drafts and the Outbox must tell editing,
    /// failed, unconfirmed and queued apart (#1491).
    pub send_state: Option<crate::DraftState>,
    pub send_at: Option<DateTime<Utc>>,
    pub has_attachments: bool,
    pub thread_count: u32,
    /// Empty for received mail; in Drafts and the Outbox the sender is the person themselves, so
    /// the row shows recipients.
    pub to: Vec<EmailAddress>,
}

/// One row per conversation (ADR 0015). Three counts are scoped to the folder asked for; one
/// deliberately is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    /// `None` for a message in no thread: it is shown as a conversation of one, since hiding mail
    /// on a null derived column is worse.
    pub id: Option<ThreadId>,
    /// What the row is drawn from and what opening it reads, so surfaces that take a message keep
    /// working.
    /// A reply filed in Archive is not what the Inbox row should show.
    pub representative: MessageSummary,
    pub subject: Option<String>,
    /// All of them: which survive eliding is a drawing decision, not a storage one.
    pub participants: Vec<EmailAddress>,
    /// Not scoped: the badge means the conversation's size wherever it is filed (ADR 0015 Q2).
    pub message_count: u32,
    /// Scoped, so a conversation whose only unread member is elsewhere reads as handled here.
    pub unread_count: u32,
    pub flagged: bool,
    pub has_attachments: bool,
    pub last_at: DateTime<Utc>,
    /// Read with the page, so drawing reads no body, and only for Focus's own scopes; other scopes
    /// answer `None`.
    #[serde(default)]
    pub marker: Option<MarkerSummary>,
    /// The same conversation in the person's other accounts, folded into this row (ADR 0005 Q2). A
    /// verb on the row must reach these too: one operation in each account's queue. Empty for lists
    /// that do not fold.
    #[serde(default)]
    pub copies: Vec<ThreadId>,
    /// Replies filed away since the person muted the conversation; read with the page, for Focus's
    /// own scopes only.
    #[serde(default)]
    pub muted: Option<u32>,
}

/// Which command the action key runs is the frontend's to derive from the kind and answer: this
/// crate cannot name a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerSummary {
    pub kind: MarkerKind,
    pub when: Option<MarkerWhen>,
    /// Verbatim and short. `None` for an invitation.
    pub excerpt: Option<String>,
    pub answer: Option<InviteAnswer>,
    /// The row says so and offers no answer. An event that is over needs no flag: `when` says when
    /// it ended.
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarkerKind {
    Invite,
    Question,
    Todo,
    NoReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarkerWhen {
    Event {
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    },
    Due(DateTime<Utc>),
}

/// The `-ing` states last while the reply waits in the outbox and can still be taken back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InviteAnswer {
    Accepting,
    Accepted,
    Declining,
    Declined,
}

/// Read with `surfaced()` on `Event::SurfacedChanged` and spliced in at `position`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Surfaced {
    Digest {
        delivery: crate::ids::DeliveryId,
        rule: String,
        cadence: Option<Cadence>,
        count: u32,
        senders: Vec<EmailAddress>,
        summary_line: Option<String>,
        at: DateTime<Utc>,
        position: u32,
    },
    Reminder {
        reminder: crate::ids::ReminderId,
        thread: ThreadId,
        since: DateTime<Utc>,
        representative: MessageSummary,
        at: DateTime<Utc>,
        position: u32,
    },
}

impl Surfaced {
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Surfaced::Digest { at, .. } | Surfaced::Reminder { at, .. } => *at,
        }
    }

    pub fn position(&self) -> u32 {
        match self {
            Surfaced::Digest { position, .. } | Surfaced::Reminder { position, .. } => *position,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cadence {
    Daily,
    Weekly,
    Monthly,
}

impl ThreadSummary {
    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadPage {
    pub total: u32,
    pub rows: Vec<ThreadSummary>,
}

/// Which window answers is the store's decision, not the caller's: a frontend that had to know
/// would be a second place to get ADR 0015's folders-thread / views-list line wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListPage {
    Messages(MessagePage),
    Threads(ThreadPage),
}

/// What `MailStore::rows_in` answers (#1607).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListRows {
    Messages(Vec<MessageSummary>),
    /// One row per conversation, however many of its messages were named.
    Threads(Vec<ThreadSummary>),
}

impl ListPage {
    pub fn total(&self) -> u32 {
        match self {
            ListPage::Messages(page) => page.total,
            ListPage::Threads(page) => page.total,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePage {
    pub total: u32,
    pub rows: Vec<MessageSummary>,
}

/// Carries a sentence, not a storage error, so the far side of this boundary need not know what
/// SQLite is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreError {
    message: String,
}

impl StoreError {
    /// The failure, phrased for the user. Never contains a secret.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        StoreError {
            message: message.into(),
        }
    }
}

/// One row of one query, so the sidebar's most-redrawn path reads once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftCounts {
    /// Also decides whether the Outbox row is drawn at all.
    pub outbox: u32,
    /// Drawn apart from the total so "one of these needs you" is visible without opening the
    /// folder.
    pub attention: u32,
    /// What Drafts shows: everything not in flight. Includes `attention`.
    pub drafts: u32,
}

/// Boxed because a frontend holds the store as a trait object, and `async fn` in traits is not
/// object-safe.
pub type Read<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// A trait, not a struct, so the crate owning a database and the one drawing rows need not compile
/// together: anything concrete here would put the database engine in the view layer
/// (`check-crate-boundaries.py`). Which window a scope lists itself as is the store's decision,
/// answered inside `list_page`. Every method returns a future, so no UI thread waits on the store.
pub trait MailStore: Send + Sync {
    fn list_page(&self, request: PageRequest) -> Read<'_, ListPage>;

    fn list_count(&self, scope: ListScope) -> Read<'_, u32>;

    /// For search hits, which no `ListScope` describes: ranked, cross-folder, no offset. The caller
    /// pages by slicing the ids. No total comes back; ids the store no longer knows are dropped, so
    /// the answer may be shorter than the request.
    fn message_rows(&self, ids: Vec<MessageId>) -> Read<'_, Vec<MessageSummary>>;

    /// Lets a change naming ids be patched into the rows on screen instead of re-reading the page
    /// (#1607). A store that can only read pages answers with an error and the list falls back to
    /// the page; this default does that.
    fn rows_in(&self, scope: ListScope, ids: Vec<MessageId>) -> Read<'_, ListRows> {
        let _ = (scope, ids);
        Box::pin(async { Err(StoreError::new("this store reads pages, not rows")) })
    }

    /// These messages left `mailbox`. Said before the list re-reads so a held count is kept by
    /// subtracting (#1607).
    /// Synchronous on purpose; a store with nothing to keep does nothing.
    fn note_removed(&self, mailbox: MailboxId, messages: Vec<MessageId>) {
        let _ = (mailbox, messages);
    }

    fn mailboxes(&self, account: AccountId) -> Read<'_, Vec<Mailbox>>;

    /// Separate from `mailboxes` because the Outbox has no row there to carry a count, and the
    /// Drafts badge needs a number the cached column does not hold.
    fn draft_counts(&self, account: AccountId) -> Read<'_, DraftCounts>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 27, hour, 0, 0).unwrap()
    }

    fn row(marker: Option<MarkerSummary>) -> ThreadSummary {
        ThreadSummary {
            id: Some(ThreadId::new(7)),
            representative: MessageSummary {
                id: MessageId::new(70),
                thread: Some(ThreadId::new(7)),
                from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
                subject: Some("Tide gate".to_owned()),
                preview: None,
                received_at: at(9),
                seen: false,
                flagged: false,
                answered: false,
                send_state: None,
                send_at: None,
                has_attachments: false,
                thread_count: 2,
                to: Vec::new(),
            },
            subject: Some("Tide gate".to_owned()),
            participants: Vec::new(),
            message_count: 2,
            unread_count: 1,
            flagged: false,
            has_attachments: false,
            last_at: at(9),
            marker,
            copies: Vec::new(),
            muted: None,
        }
    }

    #[test]
    fn a_row_written_without_a_marker_reads_back_with_none() {
        // `marker` defaults on the way in: a row from a side that never heard of markers still
        // reads.
        let mut written = serde_json::to_value(row(None)).expect("a row serialises");
        written
            .as_object_mut()
            .expect("a row is an object")
            .remove("marker")
            .expect("the field was written");
        let read: ThreadSummary = serde_json::from_value(written).expect("and reads back");
        assert_eq!(read, row(None));
    }

    #[test]
    fn a_marker_crosses_the_boundary_whole() {
        let marker = MarkerSummary {
            kind: MarkerKind::Invite,
            when: Some(MarkerWhen::Event {
                starts_at: at(14),
                ends_at: at(15),
            }),
            excerpt: None,
            answer: Some(InviteAnswer::Accepting),
            cancelled: false,
        };
        let written = serde_json::to_string(&row(Some(marker.clone()))).expect("serialises");
        let read: ThreadSummary = serde_json::from_str(&written).expect("reads back");
        assert_eq!(read.marker, Some(marker));
    }
}
