//! One row of the terminal's message list.
//!
//! Built once when its page arrives, from the store's row, with every string
//! that came from mail already made [`SafeText`] -- so nothing the list draws
//! can carry an escape sequence to the terminal (research R5). The list
//! itself is `postio_ui::list::ListWindow`, the same window the desktop app
//! and the macOS frontend page through.

use crate::app::Placement;
use chrono::{DateTime, Utc};
use postio_model::listing::{ListPage, MarkerSummary, MessageSummary, ThreadSummary};
use postio_model::{DraftState, MessageId, ThreadId};
use postio_ui::focus_list::{Conversation, FocusRow};
use postio_ui::label_colour::{Rgb, label_colour};
use postio_ui::list::ListRow;
use postio_ui::paging::{Page, PageRequest};
use postio_ui::terminal::SafeText;

/// What a row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A conversation, or a message listed on its own.
    Message,
    /// A reminder that found no reply: a conversation, spliced where it
    /// came due.
    Reminder,
    /// A digest delivery, standing for many messages.
    Digest,
}

/// A label's pill: its name and the colour of its dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pill {
    /// The label's name.
    pub name: SafeText,
    /// The colour of the `●` before it.
    pub colour: Rgb,
}

/// One row: a message, or a conversation standing for its newest message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The message this row opens. A digest's is its delivery's id negated,
    /// which no message's can be.
    pub id: MessageId,
    /// Its conversation, when it has one.
    pub thread: Option<ThreadId>,
    /// Whether the row stands for the whole conversation.
    pub is_thread: bool,
    /// What the row stands for.
    pub kind: Kind,
    /// Who it is from: a name, or else the address; a digest's cadence.
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
    /// Whether it has an attachment.
    pub attachment: bool,
    /// How many messages the conversation holds; one for a message.
    pub count: u32,
    /// What the conversation is waiting on, with its sentence already safe
    /// to draw.
    pub marker: Option<MarkerSummary>,
    /// Its labels, in the order they were made; the first two are pills.
    pub labels: Vec<Pill>,
    /// A draft's sending state, in Drafts and the Outbox.
    pub send_state: Option<DraftState>,
}

impl Row {
    /// The local day the row's mail arrived on: what its heading names.
    pub fn day(&self) -> chrono::NaiveDate {
        self.when.with_timezone(&chrono::Local).date_naive()
    }

    /// Whether the row draws a second line: a conversation with a marker.
    /// Its kind decides its height, never its content.
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
    /// A conversation's row, with its labels.
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
                // The quoted sentence is the sender's.
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

/// The run of the store's conversations a page of positions needs: the
/// positions themselves, less the surfaced rows spliced among them.
pub fn store_range(request: &PageRequest, placement: Option<&Placement>) -> (u32, u32) {
    match placement {
        Some(placement) => {
            let store = placement
                .spliced
                .page(request.offset, request.limit, u32::MAX);
            (store.offset, store.limit.max(1))
        }
        None => (request.offset, request.limit),
    }
}

/// A page the store answered, as rows. `labelled` is every `(thread, label)`
/// pair its conversations hold; `placement` puts Focus's surfaced rows among
/// them, and the page's total then counts them.
pub fn page_of(
    request: &PageRequest,
    page: ListPage,
    labelled: Vec<(ThreadId, postio_model::Label)>,
    placement: Option<Placement>,
) -> Page<Row> {
    match page {
        ListPage::Messages(page) => Page {
            total: page.total,
            rows: page.rows.into_iter().map(Row::from).collect(),
        },
        ListPage::Threads(page) => {
            let stored_total = page.total;
            let stored = postio_ui::focus_list::conversations(page.rows, labelled);
            let Some(placement) = placement else {
                return Page {
                    total: stored_total,
                    rows: stored.into_iter().map(Row::from).collect(),
                };
            };
            let placed = placement
                .spliced
                .page(request.offset, request.limit, stored_total);
            Page {
                total: placement.spliced.total(stored_total),
                rows: postio_ui::focus_list::place(&placed.slots, &placement.surfaced, &stored)
                    .into_iter()
                    .map(Row::from)
                    .collect(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use postio_model::listing::{ListPage, ThreadPage};
    use postio_model::{FocusScope, ListScope};
    use postio_ui::focus_list::FocusRow;
    use postio_ui::surfaced::Spliced;

    use super::*;
    use crate::test_support::{conversation, label, local};

    fn request(offset: u32, limit: u32) -> PageRequest {
        PageRequest {
            scope: ListScope::Focus(FocusScope::Inbox),
            page: 0,
            offset,
            limit,
        }
    }

    fn digest_row() -> FocusRow {
        FocusRow::Digest(postio_ui::focus_list::Digest {
            delivery: postio_model::ids::DeliveryId::new(2),
            rule: "Newsletters".into(),
            cadence: None,
            count: 3,
            senders: Vec::new(),
            summary_line: None,
            at: local(23, 9, 0),
        })
    }

    #[test]
    fn a_page_is_placed_around_the_surfaced_rows_and_labelled_once() {
        // The digest sits after one conversation.
        let placement = Placement {
            surfaced: vec![digest_row()],
            spliced: Spliced::new(&[1]),
        };
        assert_eq!(
            store_range(&request(0, 50), Some(&placement)),
            (0, 49),
            "the store is asked for the positions less the digest"
        );
        let rows = vec![
            conversation(1, "Ada", "One", "", local(23, 11, 0)),
            conversation(2, "Bea", "Two", "", local(23, 10, 0)),
        ];
        let page = page_of(
            &request(0, 50),
            ListPage::Threads(ThreadPage { total: 2, rows }),
            vec![
                (ThreadId::new(2), label(7, "Harbor")),
                (ThreadId::new(1), label(8, "Atlas")),
            ],
            Some(placement),
        );
        assert_eq!(page.total, 3, "the digest counts");
        let kinds: Vec<Kind> = page.rows.iter().map(|row| row.kind).collect();
        assert_eq!(kinds, [Kind::Message, Kind::Digest, Kind::Message]);
        let pill = |row: &Row| {
            row.labels
                .iter()
                .map(|pill| pill.name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(pill(&page.rows[0]), ["Atlas"]);
        assert_eq!(pill(&page.rows[2]), ["Harbor"]);
        assert!(page.rows[1].id.get() < 0, "a digest is no message's id");
    }

    #[test]
    fn a_list_with_nothing_surfaced_is_the_stores_pages() {
        assert_eq!(store_range(&request(50, 50), None), (50, 50));
        let page = page_of(
            &request(0, 50),
            ListPage::Threads(ThreadPage {
                total: 1,
                rows: vec![conversation(1, "Ada", "One", "", local(23, 11, 0))],
            }),
            Vec::new(),
            None,
        );
        assert_eq!((page.total, page.rows.len()), (1, 1));
    }
}
