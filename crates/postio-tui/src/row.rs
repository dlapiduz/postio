//! One row of the terminal's message list.
//!
//! Built once when its page arrives, from the store's row, with every string
//! that came from mail already made [`SafeText`] -- so nothing the list draws
//! can carry an escape sequence to the terminal (research R5). The list
//! itself is `postio_ui::list::ListWindow`, the same window the desktop app
//! and the macOS frontend page through.

use chrono::{DateTime, Utc};
use postio_model::listing::{ListPage, MessageSummary, ThreadSummary};
use postio_model::{MessageId, ThreadId};
use postio_ui::list::ListRow;
use postio_ui::paging::Page;
use postio_ui::terminal::SafeText;

/// One row: a message, or a conversation standing for its newest message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The message this row opens.
    pub id: MessageId,
    /// Its conversation, when it has one.
    pub thread: Option<ThreadId>,
    /// Whether the row stands for the whole conversation.
    pub is_thread: bool,
    /// Who it is from: a name, or else the address.
    pub from: SafeText,
    /// The sender's address, for what is decided by sender: remote images.
    pub address: Option<String>,
    /// The subject.
    pub subject: SafeText,
    /// The snippet after the subject.
    pub preview: SafeText,
    /// When it arrived, or the conversation last moved.
    pub when: DateTime<Utc>,
    /// Whether anything in it is unread.
    pub unread: bool,
    /// Whether it is flagged.
    pub flagged: bool,
    /// Whether it has an attachment.
    pub attachment: bool,
    /// How many messages the conversation holds; one for a message.
    pub count: u32,
}

impl From<MessageSummary> for Row {
    fn from(message: MessageSummary) -> Row {
        Row {
            id: message.id,
            thread: message.thread,
            is_thread: false,
            from: SafeText::new(message.from.as_ref().map_or("", |from| from.display())),
            address: message.from.as_ref().map(|from| from.address.clone()),
            subject: SafeText::new(message.subject.as_deref().unwrap_or("")),
            preview: SafeText::new(message.preview.as_deref().unwrap_or("")),
            when: message.received_at,
            unread: !message.seen,
            flagged: message.flagged,
            attachment: message.has_attachments,
            count: message.thread_count.max(1),
        }
    }
}

impl From<ThreadSummary> for Row {
    fn from(thread: ThreadSummary) -> Row {
        let names: Vec<&str> = thread
            .participants
            .iter()
            .map(|who| who.display())
            .collect();
        let from = if names.is_empty() {
            thread
                .representative
                .from
                .as_ref()
                .map_or(String::new(), |from| from.display().to_owned())
        } else {
            names.join(", ")
        };
        let subject = thread
            .subject
            .clone()
            .or_else(|| thread.representative.subject.clone())
            .unwrap_or_default();
        Row {
            id: thread.representative.id,
            thread: thread.id,
            is_thread: thread.id.is_some(),
            from: SafeText::new(&from),
            address: thread
                .representative
                .from
                .as_ref()
                .map(|from| from.address.clone()),
            subject: SafeText::new(&subject),
            preview: SafeText::new(thread.representative.preview.as_deref().unwrap_or("")),
            when: thread.last_at,
            unread: thread.has_unread(),
            flagged: thread.flagged,
            attachment: thread.has_attachments,
            count: thread.message_count.max(1),
        }
    }
}

impl ListRow for Row {
    fn id(&self) -> Option<MessageId> {
        Some(self.id)
    }

    fn thread(&self) -> Option<ThreadId> {
        self.is_thread.then_some(self.thread).flatten()
    }
}

/// A page from the store, as rows.
pub fn page_of(page: ListPage) -> Page<Row> {
    match page {
        ListPage::Messages(page) => Page {
            total: page.total,
            rows: page.rows.into_iter().map(Row::from).collect(),
        },
        ListPage::Threads(page) => Page {
            total: page.total,
            rows: page.rows.into_iter().map(Row::from).collect(),
        },
    }
}
