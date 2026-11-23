//! What one position of Focus's list stands for and how store pages become rows, with no toolkit.

use chrono::{DateTime, Utc};
use postio_model::ids::{DeliveryId, MessageId, ReminderId, ThreadId};
use postio_model::listing::MessageSummary;
use postio_model::listing::{
    Cadence, MarkerKind, MarkerSummary, MarkerWhen, Surfaced, ThreadSummary,
};
use postio_model::{EmailAddress, Label};

use crate::surfaced::Slot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusRow {
    Conversation(Conversation),
    Reminder {
        reminder: ReminderId,
        row: Conversation,
    },
    Digest(Digest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub delivery: DeliveryId,
    pub rule: String,
    pub cadence: Option<Cadence>,
    pub count: u32,
    pub senders: Vec<EmailAddress>,
    pub summary_line: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub summary: ThreadSummary,
    pub labels: Vec<Label>,
}

impl FocusRow {
    pub fn conversation(summary: ThreadSummary) -> Self {
        FocusRow::Conversation(Conversation {
            summary,
            labels: Vec::new(),
        })
    }

    pub fn surfaced(surfaced: &Surfaced) -> Option<Self> {
        if let Surfaced::Digest {
            delivery,
            rule,
            cadence,
            count,
            senders,
            summary_line,
            at,
            ..
        } = surfaced
        {
            return Some(FocusRow::Digest(Digest {
                delivery: *delivery,
                rule: rule.clone(),
                cadence: *cadence,
                count: *count,
                senders: senders.clone(),
                summary_line: summary_line.clone(),
                at: *at,
            }));
        }
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
                clash: None,
                moved_from: None,
            }),
            copies: Vec::new(),
            muted: None,
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

    pub fn as_conversation(&self) -> Option<&Conversation> {
        match self {
            FocusRow::Conversation(row) | FocusRow::Reminder { row, .. } => Some(row),
            FocusRow::Digest(_) => None,
        }
    }

    /// A digest stands for no one message; its row is named by its delivery, negated, which no
    /// message id can be.
    pub fn id(&self) -> MessageId {
        match self {
            FocusRow::Digest(digest) => MessageId::new(-digest.delivery.get()),
            FocusRow::Conversation(row) | FocusRow::Reminder { row, .. } => {
                row.summary.representative.id
            }
        }
    }

    /// Identity across re-reads: a conversation's is its thread; a reminder's and a digest's are
    /// their own.
    pub fn key(&self) -> crate::list::RowKey {
        use crate::list::RowKey;
        match self {
            FocusRow::Digest(digest) => RowKey::Other(0, digest.delivery.get()),
            FocusRow::Reminder { reminder, .. } => RowKey::Other(1, reminder.get()),
            FocusRow::Conversation(row) => row.summary.id.map_or(
                RowKey::Message(row.summary.representative.id),
                RowKey::Thread,
            ),
        }
    }

    pub fn thread(&self) -> Option<ThreadId> {
        self.as_conversation().and_then(|row| row.summary.id)
    }

    pub fn threads(&self) -> Vec<ThreadId> {
        let Some(row) = self.as_conversation() else {
            return Vec::new();
        };
        row.summary
            .id
            .into_iter()
            .chain(row.summary.copies.iter().copied())
            .collect()
    }

    /// Its kind decides its height, never its content.
    pub fn two_lines(&self) -> bool {
        self.as_conversation()
            .is_some_and(|row| row.summary.marker.is_some())
    }

    pub fn at(&self) -> DateTime<Utc> {
        match self {
            FocusRow::Digest(digest) => digest.at,
            FocusRow::Conversation(row) | FocusRow::Reminder { row, .. } => row.summary.last_at,
        }
    }
}

pub fn day_of(row: &FocusRow) -> chrono::NaiveDate {
    row.at().with_timezone(&chrono::Local).date_naive()
}

/// `labelled` is every `(thread, label)` pair for the page, copies included; a row holds each label
/// once.
pub fn conversations(
    summaries: Vec<ThreadSummary>,
    mut labelled: Vec<(ThreadId, Label)>,
) -> Vec<FocusRow> {
    summaries
        .into_iter()
        .map(|summary| {
            let mine: Vec<ThreadId> = summary
                .id
                .into_iter()
                .chain(summary.copies.iter().copied())
                .collect();
            let mut labels: Vec<Label> = Vec::new();
            labelled.retain(|(thread, label)| {
                if !mine.contains(thread) {
                    return true;
                }
                if !labels.iter().any(|held| held.id == label.id) {
                    labels.push(label.clone());
                }
                false
            });
            FocusRow::Conversation(Conversation { summary, labels })
        })
        .collect()
}

pub fn label_threads(summaries: &[ThreadSummary]) -> Vec<ThreadId> {
    summaries
        .iter()
        .flat_map(|row| row.id.into_iter().chain(row.copies.iter().copied()))
        .collect()
}

pub fn lone(message: MessageSummary) -> ThreadSummary {
    ThreadSummary {
        id: message.thread,
        subject: message.subject.clone(),
        participants: message.from.iter().cloned().collect(),
        message_count: 1,
        unread_count: u32::from(!message.seen),
        flagged: message.flagged,
        has_attachments: message.has_attachments,
        last_at: message.received_at,
        marker: None,
        copies: Vec::new(),
        muted: None,
        representative: message,
    }
}

/// A slot naming a row that is not there is skipped.
pub fn place(slots: &[Slot], surfaced: &[FocusRow], stored: &[FocusRow]) -> Vec<FocusRow> {
    slots
        .iter()
        .filter_map(|slot| match slot {
            Slot::Surfaced(index) => surfaced.get(*index).cloned(),
            Slot::Stored(index) => stored.get(*index).cloned(),
        })
        .collect()
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
                to: Vec::new(),
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
            muted: None,
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
            clash: None,
            moved_from: None,
        });
        assert!(FocusRow::conversation(marked).two_lines());
    }

    fn label(id: i64) -> Label {
        let mut label = Label::new(postio_model::ids::AccountId::new(1), format!("l{id}"));
        label.id = postio_model::ids::LabelId::new(id);
        label
    }

    #[test]
    fn a_row_holds_each_label_once_across_its_copies() {
        let mut folded = conversation(1, Some(3));
        folded.copies = vec![ThreadId::new(9)];
        let other = conversation(2, Some(4));
        let rows = conversations(
            vec![folded, other],
            vec![
                (ThreadId::new(3), label(1)),
                (ThreadId::new(4), label(2)),
                (ThreadId::new(9), label(1)),
                (ThreadId::new(9), label(5)),
            ],
        );
        let labels = |row: &FocusRow| {
            row.as_conversation()
                .expect("a conversation")
                .labels
                .iter()
                .map(|label| label.id.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(&rows[0]), vec![1, 5]);
        assert_eq!(labels(&rows[1]), vec![2]);
    }

    #[test]
    fn a_page_is_labelled_for_every_thread_it_folds() {
        let mut folded = conversation(1, Some(3));
        folded.copies = vec![ThreadId::new(9)];
        assert_eq!(
            label_threads(&[folded, conversation(2, None)]),
            vec![ThreadId::new(3), ThreadId::new(9)]
        );
    }

    #[test]
    fn a_draft_is_a_conversation_of_one() {
        let message = conversation(5, None).representative;
        let lone = lone(message);
        assert_eq!(lone.message_count, 1);
        assert_eq!(lone.unread_count, 1);
        assert!(lone.marker.is_none());
    }

    #[test]
    fn a_spliced_page_reads_surfaced_and_stored_rows_in_slot_order() {
        let surfaced = vec![FocusRow::conversation(conversation(100, Some(100)))];
        let stored = vec![
            FocusRow::conversation(conversation(1, Some(1))),
            FocusRow::conversation(conversation(2, Some(2))),
        ];
        let rows = place(
            &[
                Slot::Stored(0),
                Slot::Surfaced(0),
                Slot::Stored(1),
                Slot::Stored(7),
            ],
            &surfaced,
            &stored,
        );
        let ids: Vec<i64> = rows.iter().map(|row| row.id().get()).collect();
        assert_eq!(ids, vec![1, 100, 2]);
    }
}
