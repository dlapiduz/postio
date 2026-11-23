//! The message list, windowed: a count and one row at a time, synchronously, with pages arriving
//! behind (PRODUCT.md §18).

use postio_model::{MessageId, ThreadId};
use postio_runtime::store::{ListPage, ListScope, MessageSummary, ThreadSummary};
use postio_ui::list::ListRow;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ScopeFfi {
    Mailbox {
        mailbox: i64,
    },
    Account {
        account: i64,
    },
    /// Every folder in every enabled account at once (ADR 0005 Q4): a view, so it names no account.
    Unified,
    Flagged {
        account: i64,
    },
    Snoozed {
        account: i64,
    },
    /// The sidebar's "Outbox" view: drafts whose send is under way.
    Outbox {
        account: i64,
    },
    /// One conversation, wherever its messages are filed: a thread spans folders, so it is not a
    /// narrowing of a mailbox.
    Thread {
        thread: i64,
    },
}

impl From<ScopeFfi> for ListScope {
    fn from(scope: ScopeFfi) -> Self {
        match scope {
            ScopeFfi::Mailbox { mailbox } => ListScope::Mailbox(mailbox.into()),
            ScopeFfi::Account { account } => ListScope::Account(account.into()),
            ScopeFfi::Unified => ListScope::Unified,
            ScopeFfi::Flagged { account } => ListScope::Flagged(account.into()),
            ScopeFfi::Snoozed { account } => ListScope::Snoozed(account.into()),
            ScopeFfi::Outbox { account } => ListScope::Outbox(account.into()),
            ScopeFfi::Thread { thread } => ListScope::Thread(thread.into()),
        }
    }
}

/// One row of the list. Message rows and conversation rows share a type: a folder shows
/// conversations, a query view messages (ADR 0015).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowFfi {
    /// The message this row opens. For a conversation row, its representative.
    pub id: i64,
    /// Set for a message in a thread as well as for a row standing for one; see `is_thread`.
    pub thread: Option<i64>,
    /// Whether this row stands for a whole conversation: a thread row's `id` is its newest message,
    /// so verbs cannot otherwise tell the two apart (#468).
    pub is_thread: bool,
    pub from: Option<String>,
    /// The sender's address, unrendered: `hello@pinepoint-radon.example`. A per-sender privacy
    /// decision is made about this, never a display name.
    pub from_address: Option<String>,
    /// The two letters of the avatar chip, from `postio_ui::row::initials` so both platforms
    /// abbreviate alike.
    pub initials: String,
    pub subject: Option<String>,
    pub preview: Option<String>,
    /// The sort key, as seconds since the epoch.
    pub received_at: i64,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    /// The draft's send state as the stored spelling (`"editing"`, `"queued"`, `"sending"`,
    /// `"failed"`, `"unconfirmed"`), or `None` for ordinary mail. A string, not a mirrored enum:
    /// the frontend only renders it.
    pub send_state: Option<String>,
    pub has_attachments: bool,
    /// How many messages the conversation holds; the badge appears above one.
    pub thread_count: u32,
    /// Who is in the conversation, elided: `Tessa, Mara, Pinepoint`. Empty on a message row, which
    /// is how the two are told apart.
    pub participants: String,
}

impl ListRow for RowFfi {
    fn id(&self) -> Option<MessageId> {
        Some(self.id.into())
    }

    fn thread(&self) -> Option<ThreadId> {
        // Both conditions, as the GTK row applies them: a conversation row with no thread id is not
        // one verbs can name, and a message row in a thread is not standing for it.
        self.is_thread
            .then(|| self.thread.map(Into::into))
            .flatten()
    }
}

impl postio_ui::conversation::ConversationMessage for RowFfi {
    fn seen(&self) -> bool {
        self.seen
    }

    fn received_at(&self) -> chrono::DateTime<chrono::Utc> {
        // Seconds cross the C ABI; an unrepresentable stamp sorts as the epoch rather than
        // panicking a redraw.
        chrono::DateTime::from_timestamp(self.received_at, 0).unwrap_or_default()
    }

    fn ordinal(&self) -> i64 {
        self.id
    }
}

impl From<MessageSummary> for RowFfi {
    fn from(row: MessageSummary) -> Self {
        RowFfi {
            id: row.id.into(),
            thread: row.thread.map(Into::into),
            // `display()`, not `to_string()`: the RFC form spends the line on an address and
            // truncates the name (#1150).
            initials: postio_ui::row::initials(row.from.as_ref()),
            from_address: row.from.as_ref().map(|address| address.address.clone()),
            from: row.from.map(|address| address.display().to_string()),
            subject: row.subject,
            preview: row.preview,
            received_at: row.received_at.timestamp(),
            seen: row.seen,
            flagged: row.flagged,
            answered: row.answered,
            // The reader's word, not the database's spelling: "Not sent" because what matters is
            // that it did not go (#1487);
            // "Not confirmed" because nobody can tell whether it arrived (ADR 0021 D3).
            send_state: row
                .send_state
                .map(|state| postio_ui::row::send_state_word(state).to_owned()),
            has_attachments: row.has_attachments,
            thread_count: row.thread_count,
            is_thread: false,
            participants: String::new(),
        }
    }
}

impl From<ThreadSummary> for RowFfi {
    fn from(row: ThreadSummary) -> Self {
        // Where the representative and the thread disagree the conversation wins: that is what the
        // row stands for.
        let mut base = RowFfi::from(row.representative);
        base.thread = row.id.map(Into::into);
        if row.subject.is_some() {
            base.subject = row.subject;
        }
        base.flagged = row.flagged;
        base.has_attachments = row.has_attachments;
        base.received_at = row.last_at.timestamp();
        // Scoped, unlike `message_count`: unread is what you act on from this folder.
        base.seen = row.unread_count == 0;
        base.thread_count = row.message_count;
        base.is_thread = true;
        base.participants = postio_ui::conversation::participants(&row.participants);
        base
    }
}

/// One page as the window takes it. The total travels with the rows, from one read, so count and
/// page cannot disagree.
pub fn page_of(page: ListPage) -> postio_ui::paging::Page<RowFfi> {
    match page {
        ListPage::Messages(page) => postio_ui::paging::Page {
            total: page.total,
            rows: page.rows.into_iter().map(RowFfi::from).collect(),
        },
        ListPage::Threads(page) => postio_ui::paging::Page {
            total: page.total,
            rows: page.rows.into_iter().map(RowFfi::from).collect(),
        },
    }
}
