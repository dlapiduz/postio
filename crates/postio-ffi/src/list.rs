//! The message list, windowed.
//!
//! The frontend gets a count and one row at a time, synchronously, and pages
//! arrive behind it. That shape is not an implementation detail: it is what
//! `PRODUCT.md` §18 requires — *a mailbox is never loaded into memory* — held
//! across an FFI, where handing the caller a vector would be the obvious and
//! wrong thing to do.

use postio_model::MessageId;
use postio_runtime::store::{ListPage, ListScope, MessageSummary, ThreadSummary};
use postio_ui::list::ListRow;

/// Which messages the list is showing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ScopeFfi {
    /// One folder.
    Mailbox {
        /// The folder.
        mailbox: i64,
    },
    /// Every folder in an account: the unified view.
    Account {
        /// The account.
        account: i64,
    },
    /// The sidebar's "Flagged" view.
    Flagged {
        /// The account.
        account: i64,
    },
    /// The sidebar's "Snoozed" view.
    Snoozed {
        /// The account.
        account: i64,
    },
}

impl From<ScopeFfi> for ListScope {
    fn from(scope: ScopeFfi) -> Self {
        match scope {
            ScopeFfi::Mailbox { mailbox } => ListScope::Mailbox(mailbox.into()),
            ScopeFfi::Account { account } => ListScope::Account(account.into()),
            ScopeFfi::Flagged { account } => ListScope::Flagged(account.into()),
            ScopeFfi::Snoozed { account } => ListScope::Snoozed(account.into()),
        }
    }
}

/// One row of the list, however its scope lists itself.
///
/// One type for both message rows and conversation rows, because a folder
/// shows conversations and a query view shows messages (ADR 0015) and a
/// frontend that understood only one would draw the wrong thing in half the
/// application. A conversation row is its representative message plus the
/// conversation's own subject and count — which is what the GTK row draws too.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowFfi {
    /// The message this row opens. For a conversation row, its representative.
    pub id: i64,
    /// The conversation, when the row stands for one.
    pub thread: Option<i64>,
    /// Who it is from, already rendered for display.
    pub from: Option<String>,
    /// The subject: the conversation's when there is one, else the message's.
    pub subject: Option<String>,
    /// The snippet under the subject.
    pub preview: Option<String>,
    /// The sort key, as seconds since the epoch.
    pub received_at: i64,
    /// Whether it has been read.
    pub seen: bool,
    /// Whether it carries `\Flagged`.
    pub flagged: bool,
    /// Whether it has been replied to.
    pub answered: bool,
    /// Whether it is a draft.
    pub draft: bool,
    /// Whether it has an attachment.
    pub has_attachments: bool,
    /// How many messages the conversation holds; the badge appears above one.
    pub thread_count: u32,
}

impl ListRow for RowFfi {
    fn id(&self) -> Option<MessageId> {
        Some(self.id.into())
    }
}

impl From<MessageSummary> for RowFfi {
    fn from(row: MessageSummary) -> Self {
        RowFfi {
            id: row.id.into(),
            thread: row.thread.map(Into::into),
            from: row.from.map(|address| address.to_string()),
            subject: row.subject,
            preview: row.preview,
            received_at: row.received_at.timestamp(),
            seen: row.seen,
            flagged: row.flagged,
            answered: row.answered,
            draft: row.draft,
            has_attachments: row.has_attachments,
            thread_count: row.thread_count,
        }
    }
}

impl From<ThreadSummary> for RowFfi {
    fn from(row: ThreadSummary) -> Self {
        // The representative carries the message-level truth; the thread
        // carries the conversation-level truth. Where they disagree the
        // conversation wins, because that is what the row is standing for.
        let mut base = RowFfi::from(row.representative);
        base.thread = row.id.map(Into::into);
        if row.subject.is_some() {
            base.subject = row.subject;
        }
        base.flagged = row.flagged;
        base.has_attachments = row.has_attachments;
        base.received_at = row.last_at.timestamp();
        // Scoped, unlike `message_count`: unread is what you act on from this
        // folder, so a conversation whose only unread member is filed
        // elsewhere reads as handled here.
        base.seen = row.unread_count == 0;
        base.thread_count = row.message_count;
        base
    }
}

/// The rows of one page, whichever way the scope lists itself.
pub fn rows_of(page: ListPage) -> Vec<RowFfi> {
    match page {
        ListPage::Messages(page) => page.rows.into_iter().map(RowFfi::from).collect(),
        ListPage::Threads(page) => page.rows.into_iter().map(RowFfi::from).collect(),
    }
}
