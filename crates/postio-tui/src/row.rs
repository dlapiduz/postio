//! One row of the message list, built once when its page arrives. Every string that came from mail
//! is already `SafeText`, so nothing drawn can carry an escape sequence to the terminal.

use chrono::{DateTime, Utc};
use postio_model::listing::{MarkerSummary, MessageSummary, ThreadSummary};
use postio_model::{DraftState, MessageId, ThreadId};
use postio_ui::focus_list::{Conversation, FocusRow};
use postio_ui::label_colour::{Rgb, label_colour};
use postio_ui::list::ListRow;
use postio_ui::terminal::SafeText;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Message,
    Reminder,
    Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    pub name: SafeText,
    pub colour: Rgb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The message this row opens. A digest's is its delivery's id negated,
    /// which no message's can be.
    pub id: MessageId,
    pub thread: Option<ThreadId>,
    pub is_thread: bool,
    pub kind: Kind,
    pub from: SafeText,
    pub address: Option<String>,
    pub subject: SafeText,
    pub preview: SafeText,
    pub when: DateTime<Utc>,
    pub unread: bool,
    pub attachment: bool,
    pub count: u32,
    pub marker: Option<MarkerSummary>,
    /// Its labels, in the order they were made; the first two are pills.
    pub labels: Vec<Pill>,
    pub send_state: Option<DraftState>,
}

impl Row {
    pub fn day(&self) -> chrono::NaiveDate {
        self.when.with_timezone(&chrono::Local).date_naive()
    }

    pub fn two_lines(&self) -> bool {
        self.marker.is_some()
    }
}

/// The hue the accent has, which no label's colour may come near.
pub(crate) fn accent_hue() -> f64 {
    let (_, dark) = postio_ui::tokens::accent_rgb();
    Rgb::new(dark.0, dark.1, dark.2).hue()
}

impl From<MessageSummary> for Row {
    fn from(message: MessageSummary) -> Row {
        Row {
            id: message.id,
            thread: message.thread,
            is_thread: false,
            kind: Kind::Message,
            from: SafeText::new(message.from.as_ref().map_or("", |from| from.display())),
            address: message.from.as_ref().map(|from| from.address.clone()),
            subject: SafeText::new(message.subject.as_deref().unwrap_or("")),
            preview: SafeText::new(message.preview.as_deref().unwrap_or("")),
            when: message.received_at,
            unread: !message.seen,
            attachment: message.has_attachments,
            count: message.thread_count.max(1),
            marker: None,
            labels: Vec::new(),
            send_state: message.send_state,
        }
    }
}

impl From<ThreadSummary> for Row {
    fn from(thread: ThreadSummary) -> Row {
        Row::conversation(Conversation {
            summary: thread,
            labels: Vec::new(),
        })
    }
}

impl Row {
    fn conversation(conversation: Conversation) -> Row {
        let Conversation { summary, labels } = conversation;
        let names: Vec<&str> = summary
            .participants
            .iter()
            .map(|who| who.display())
            .collect();
        let from = if names.is_empty() {
            summary
                .representative
                .from
                .as_ref()
                .map_or(String::new(), |from| from.display().to_owned())
        } else {
            names.join(", ")
        };
        let subject = summary
            .subject
            .clone()
            .or_else(|| summary.representative.subject.clone())
            .unwrap_or_default();
        let hue = accent_hue();
        Row {
            id: summary.representative.id,
            thread: summary.id,
            is_thread: summary.id.is_some(),
            kind: Kind::Message,
            from: SafeText::new(&from),
            address: summary
                .representative
                .from
                .as_ref()
                .map(|from| from.address.clone()),
            subject: SafeText::new(&subject),
            preview: SafeText::new(summary.representative.preview.as_deref().unwrap_or("")),
            when: summary.last_at,
            unread: summary.has_unread(),
            attachment: summary.has_attachments,
            count: summary.message_count.max(1),
            marker: summary.marker.map(|mut marker| {
                marker.excerpt = marker
                    .excerpt
                    .map(|excerpt| SafeText::new(&excerpt).as_str().to_owned());
                marker
            }),
            labels: labels
                .iter()
                .map(|label| Pill {
                    name: SafeText::new(&label.name),
                    colour: label_colour(
                        &label.name,
                        label.color.as_deref().and_then(Rgb::from_hex),
                        hue,
                    ),
                })
                .collect(),
            send_state: summary.representative.send_state,
        }
    }
}

impl From<FocusRow> for Row {
    fn from(row: FocusRow) -> Row {
        match row {
            FocusRow::Conversation(conversation) => Row::conversation(conversation),
            FocusRow::Reminder { row, .. } => Row {
                kind: Kind::Reminder,
                ..Row::conversation(row)
            },
            FocusRow::Digest(digest) => {
                let senders: Vec<String> = digest
                    .senders
                    .iter()
                    .map(|sender| sender.display().to_owned())
                    .collect();
                Row {
                    id: MessageId::new(-digest.delivery.get()),
                    thread: None,
                    is_thread: false,
                    kind: Kind::Digest,
                    from: SafeText::new(&postio_ui::focus_row::digest_title(digest.cadence)),
                    address: None,
                    subject: SafeText::new(&postio_ui::focus_row::digest_subject(
                        &digest.rule,
                        digest.count,
                    )),
                    preview: SafeText::new(&postio_ui::focus_row::digest_line(
                        digest.summary_line.as_deref(),
                        &senders,
                    )),
                    when: digest.at,
                    unread: false,
                    attachment: false,
                    count: digest.count,
                    marker: None,
                    labels: Vec::new(),
                    send_state: None,
                }
            }
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
