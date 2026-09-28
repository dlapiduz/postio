//! What one position of Focus's list stands for, with no toolkit in it.

use postio_model::ids::{MessageId, ThreadId};
use postio_model::listing::ThreadSummary;

/// One row of Focus's list.
///
/// A conversation today. Digest deliveries and fired reminders join it as
/// rows spliced among the conversations (research R3, data-model
/// `FocusRow`), which is why this is an enum from the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusRow {
    /// A conversation, drawn from its newest message in the inbox.
    Conversation(ThreadSummary),
}

impl FocusRow {
    /// The message the row stands for: the one opening it opens, and the
    /// one the selection names it by.
    pub fn id(&self) -> MessageId {
        match self {
            FocusRow::Conversation(summary) => summary.representative.id,
        }
    }

    /// The conversation, when the row stands for one.
    pub fn thread(&self) -> Option<ThreadId> {
        match self {
            FocusRow::Conversation(summary) => summary.id,
        }
    }

    /// Every conversation an action on this row must reach: its own and the
    /// copies folded into it from the person's other accounts (T161).
    pub fn threads(&self) -> Vec<ThreadId> {
        match self {
            FocusRow::Conversation(summary) => summary
                .id
                .into_iter()
                .chain(summary.copies.iter().copied())
                .collect(),
        }
    }

    /// Whether the row draws a second line: a conversation with a marker.
    /// Its kind decides its height, never its content (FR-013).
    pub fn two_lines(&self) -> bool {
        match self {
            FocusRow::Conversation(summary) => summary.marker.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use postio_model::listing::{MarkerKind, MarkerSummary, MessageSummary};

    use super::*;

    fn conversation(id: i64, thread: Option<i64>) -> ThreadSummary {
        ThreadSummary {
            id: thread.map(ThreadId::new),
            representative: MessageSummary {
                id: MessageId::new(id),
                thread: thread.map(ThreadId::new),
                from: None,
                subject: None,
                preview: None,
                received_at: Utc::now(),
                seen: false,
                flagged: false,
                answered: false,
                send_state: None,
                send_at: None,
                has_attachments: false,
                thread_count: 1,
            },
            subject: None,
            participants: Vec::new(),
            message_count: 1,
            unread_count: 1,
            flagged: false,
            has_attachments: false,
            last_at: Utc::now(),
            marker: None,
            copies: Vec::new(),
        }
    }

    #[test]
    fn a_folded_row_reaches_every_copy_of_its_conversation() {
        let mut summary = conversation(7, Some(3));
        summary.copies = vec![ThreadId::new(9)];
        let item = FocusRow::Conversation(summary);
        assert_eq!(item.id(), MessageId::new(7));
        assert_eq!(item.threads(), vec![ThreadId::new(3), ThreadId::new(9)]);
        assert_eq!(
            FocusRow::Conversation(conversation(8, None)).threads(),
            Vec::<ThreadId>::new(),
            "a message in no conversation is aimed at by its id, not a thread"
        );
    }

    #[test]
    fn a_row_is_two_lines_exactly_when_it_carries_a_marker() {
        let mut marked = conversation(1, Some(1));
        assert!(!FocusRow::Conversation(marked.clone()).two_lines());
        marked.marker = Some(MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you?".into()),
            answer: None,
            cancelled: false,
        });
        assert!(FocusRow::Conversation(marked).two_lines());
    }
}
