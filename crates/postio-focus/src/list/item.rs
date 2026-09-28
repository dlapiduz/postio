//! What one position of Focus's list stands for, with no toolkit in it.

use chrono::{DateTime, Utc};
use postio_model::Label;
use postio_model::ids::{MessageId, ReminderId, ThreadId};
use postio_model::listing::{MarkerKind, MarkerSummary, MarkerWhen, Surfaced, ThreadSummary};

/// One row of Focus's list.
///
/// A conversation, or a fired reminder spliced among them at its place
/// (research R3, data-model `FocusRow`). Digest deliveries join as rows of
/// their own too, which is why this is an enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusRow {
    /// A conversation, drawn from its newest message in the inbox.
    Conversation(Conversation),
    /// A reminder that found no reply by its time (T095): its
    /// conversation, drawn as one, marked "No reply since …" and sitting
    /// where it came due rather than where its mail did.
    Reminder {
        /// Which reminder.
        reminder: ReminderId,
        /// The conversation it stands for, with its marker.
        row: Conversation,
    },
}

/// A conversation's row: the list's summary of it, and its labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// What the list read: the newest message, the counts, the marker.
    pub summary: ThreadSummary,
    /// Its labels, in the order they were made: the first two are its
    /// pills (`postio_ui::focus_row::MAX_PILLS`).
    pub labels: Vec<Label>,
}

impl FocusRow {
    /// A conversation's row, before its labels have been read.
    pub fn conversation(summary: ThreadSummary) -> Self {
        FocusRow::Conversation(Conversation {
            summary,
            labels: Vec::new(),
        })
    }

    /// A surfaced row, when it is one this list draws: a fired reminder.
    /// Its conversation is drawn from its latest message, as of when it
    /// fired, with the "No reply" marker naming the day it was set.
    pub fn surfaced(surfaced: &Surfaced) -> Option<Self> {
        let Surfaced::Reminder {
            reminder,
            thread,
            since,
            representative,
            at,
            ..
        } = surfaced
        else {
            return None;
        };
        let summary = ThreadSummary {
            id: Some(*thread),
            subject: representative.subject.clone(),
            participants: representative.from.iter().cloned().collect(),
            message_count: representative.thread_count.max(1),
            unread_count: u32::from(!representative.seen),
            flagged: representative.flagged,
            has_attachments: representative.has_attachments,
            last_at: *at,
            marker: Some(MarkerSummary {
                kind: MarkerKind::NoReply,
                when: Some(MarkerWhen::Due(*since)),
                excerpt: None,
                answer: None,
                cancelled: false,
            }),
            copies: Vec::new(),
            representative: representative.clone(),
        };
        Some(FocusRow::Reminder {
            reminder: *reminder,
            row: Conversation {
                summary,
                labels: Vec::new(),
            },
        })
    }

    /// The conversation the row draws, whatever brought it here.
    pub fn row(&self) -> &Conversation {
        match self {
            FocusRow::Conversation(row) | FocusRow::Reminder { row, .. } => row,
        }
    }

    /// The message the row stands for: the one opening it opens, and the
    /// one the selection names it by.
    pub fn id(&self) -> MessageId {
        self.row().summary.representative.id
    }

    /// The conversation, when the row stands for one.
    pub fn thread(&self) -> Option<ThreadId> {
        self.row().summary.id
    }

    /// Every conversation an action on this row must reach: its own and the
    /// copies folded into it from the person's other accounts (T161).
    pub fn threads(&self) -> Vec<ThreadId> {
        let summary = &self.row().summary;
        summary
            .id
            .into_iter()
            .chain(summary.copies.iter().copied())
            .collect()
    }

    /// Whether the row draws a second line: a conversation with a marker.
    /// Its kind decides its height, never its content (FR-013).
    pub fn two_lines(&self) -> bool {
        self.row().summary.marker.is_some()
    }

    /// When the row's mail arrived -- or, for a reminder, when it came
    /// back -- for its day heading and its time.
    pub fn at(&self) -> DateTime<Utc> {
        self.row().summary.last_at
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use postio_model::listing::MessageSummary;

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
        let item = FocusRow::conversation(summary);
        assert_eq!(item.id(), MessageId::new(7));
        assert_eq!(item.threads(), vec![ThreadId::new(3), ThreadId::new(9)]);
        assert_eq!(
            FocusRow::conversation(conversation(8, None)).threads(),
            Vec::<ThreadId>::new(),
            "a message in no conversation is aimed at by its id, not a thread"
        );
    }

    #[test]
    fn a_row_is_two_lines_exactly_when_it_carries_a_marker() {
        let mut marked = conversation(1, Some(1));
        assert!(!FocusRow::conversation(marked.clone()).two_lines());
        marked.marker = Some(MarkerSummary {
            kind: MarkerKind::Question,
            when: None,
            excerpt: Some("Can you?".into()),
            answer: None,
            cancelled: false,
        });
        assert!(FocusRow::conversation(marked).two_lines());
    }
}
