//! What a list read answers, as data.
//!
//! These are the rows and pages a frontend draws. They were defined beside
//! the `MailStore` trait in `postio-runtime`, which is the crate that owns a
//! database, and so no frontend that must not open the store could name
//! them. Here they are plain data, serialisable, so the store's one owner can
//! send them to a frontend in another process (ADR 0041). `postio-runtime`
//! and `postio-storage` re-export them, and every existing path still
//! resolves.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ListScope;
use crate::address::EmailAddress;
use crate::ids::{MessageId, ThreadId};

/// Which rows are wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    /// The messages being listed.
    pub scope: ListScope,
    /// The first row wanted, counted from the newest.
    pub offset: u32,
    /// How many rows to read.
    pub limit: u32,
}

/// One row of the message list, as a frontend needs it.
///
/// `postio-storage` has a struct with nearly these fields; this one adds the
/// thread count the row's badge shows and drops the size, which nothing draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    /// Local id. The row's identity, across reloads and updates.
    pub id: MessageId,
    /// The thread it belongs to, for drill-in and the count badge.
    pub thread: Option<ThreadId>,
    /// Who it is from.
    pub from: Option<EmailAddress>,
    /// `Subject`, verbatim.
    pub subject: Option<String>,
    /// The snippet under the subject.
    pub preview: Option<String>,
    /// When the server received it; the list's sort key.
    pub received_at: DateTime<Utc>,
    /// Whether it has been read.
    pub seen: bool,
    /// Whether it carries `\Flagged`.
    pub flagged: bool,
    /// Whether it has been replied to.
    pub answered: bool,
    /// Whether it is a draft, and which state its send is in.
    ///
    /// Carried all the way to the row rather than flattened to a bool at this
    /// seam: Drafts holds what you are writing, what failed and what cannot be
    /// confirmed, the Outbox holds what is on its way, and a row that cannot
    /// tell them apart renders all five the same (#1491).
    pub send_state: Option<crate::DraftState>,
    /// When a scheduled send is due (spec 003 FR-007). `None` otherwise.
    pub send_at: Option<DateTime<Utc>>,
    /// Whether it has an attachment.
    pub has_attachments: bool,
    /// How many messages are in its thread; the badge appears above one.
    pub thread_count: u32,
}

/// One row of the threaded message list, as a frontend needs it.
///
/// A folder shows one row per conversation (ADR 0015), and this is that row.
/// Three of its numbers are **scoped to the folder that was asked for** and
/// one deliberately is not — see the fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    /// The conversation this row stands for.
    ///
    /// `None` for a message belonging to no thread. Threading runs on
    /// everything sync files and can still fail, and a list that hides mail
    /// because a derived column is null is worse than one that shows it
    /// ungrouped — so an unthreaded message is a conversation of one.
    pub id: Option<ThreadId>,
    /// The newest message of the conversation **in this folder**.
    ///
    /// What the row is drawn from and what opening the row reads, so every
    /// surface that already takes a message — the reading pane, drag-out,
    /// reply — keeps working without knowing this is a thread row. A reply
    /// filed in Archive is not what the Inbox row should be showing.
    pub representative: MessageSummary,
    /// The conversation's subject: the normalised root subject.
    pub subject: Option<String>,
    /// Everyone who has written in it, in first-seen order.
    ///
    /// The row elides them for width; it is given all of them because which
    /// ones survive eliding is a drawing decision, not a storage one.
    pub participants: Vec<EmailAddress>,
    /// How many messages the conversation holds, across every folder.
    ///
    /// **Not scoped.** The badge means how big the conversation is, wherever
    /// it is filed (ADR 0015 Q2).
    pub message_count: u32,
    /// How many of its messages are unread **in this folder**.
    ///
    /// Scoped, so a conversation whose only unread member is filed elsewhere
    /// reads as handled here — unread is what you act on from this folder.
    pub unread_count: u32,
    /// Whether any member **in this folder** is flagged.
    pub flagged: bool,
    /// Whether any member has an attachment.
    pub has_attachments: bool,
    /// When the conversation last moved; the list's sort key.
    pub last_at: DateTime<Utc>,
}

impl ThreadSummary {
    /// Whether anything in this folder's slice is unread.
    pub fn has_unread(&self) -> bool {
        self.unread_count > 0
    }
}

/// One page of a folder's conversations, and how many there are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadPage {
    /// How many conversations the scope matches, as of this read.
    pub total: u32,
    /// The rows themselves, most recently active first.
    pub rows: Vec<ThreadSummary>,
}

/// A page of the list, drawn however its scope lists itself.
///
/// The frontend asks for "the next page of this scope" and draws what comes
/// back. Which of the two windows answers is the **store's** decision, not the
/// caller's, because it depends on what the folder is — and a frontend that
/// had to know would be a second place for ADR 0015's folders-thread /
/// views-list line to be got wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListPage {
    /// The scope lists messages: a query view, or a folder of drafts.
    Messages(MessagePage),
    /// The scope lists conversations: an ordinary folder.
    Threads(ThreadPage),
}

/// The rows a list would show for particular messages, in the shape the
/// scope's pages use: conversations for a folder or an account, messages
/// for a drafts folder. What `MailStore::rows_in` (in `postio-runtime`) answers (#1607).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListRows {
    /// The scope lists messages, and these are the ones asked for.
    Messages(Vec<MessageSummary>),
    /// The scope lists conversations, and these are the ones the messages
    /// asked for belong to -- one row per conversation, however many of its
    /// messages were named.
    Threads(Vec<ThreadSummary>),
}

impl ListPage {
    /// How many rows the scope matches, as of this read.
    pub fn total(&self) -> u32 {
        match self {
            ListPage::Messages(page) => page.total,
            ListPage::Threads(page) => page.total,
        }
    }
}

/// One page of a mailbox, and how long the list is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePage {
    /// How many rows the scope matches, as of this read.
    pub total: u32,
    /// The rows themselves, newest first.
    pub rows: Vec<MessageSummary>,
}

/// A read that could not be answered.
///
/// Carries a sentence rather than a storage error, for the same reason the
/// rows do: whatever is on the other side of this boundary must not need to
/// know what SQLite is. The sentence is the one the user should see.
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
    /// A failure phrased for the user.
    pub fn new(message: impl Into<String>) -> Self {
        StoreError {
            message: message.into(),
        }
    }
}

/// The three numbers the sidebar needs about an account's drafts.
///
/// Together rather than separately because they come from one row of one
/// query: asking three times would be three reads on the path redrawn most
/// often, for numbers that are only ever read together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftCounts {
    /// Drafts whose send is under way — the Outbox's badge, and what decides
    /// whether that row is drawn at all (spec 003 FR-012).
    pub outbox: u32,
    /// Drafts that have stopped and need a person: failed, or unconfirmed.
    /// Drawn apart from the total so "one of these needs you" is visible
    /// without opening the folder (FR-022).
    pub attention: u32,
    /// What Drafts shows: everything not in flight. Includes `attention`.
    pub drafts: u32,
}

