//! What surrounds the body in the Mac's message window (specs/009-focus-macos
//! T067, T068): the subject, the position line, the thread chip, the labels,
//! the sender block, the marker's card, the action row and the attachments.
//!
//! Every word is composed here by the functions GTK's open message draws
//! with (`postio_ui::focus_dialog`, `focus_row`, `reader::header`,
//! `format`), so the two windows cannot say a message differently. Swift
//! lays these out and spells the keys; it words nothing.

use postio_core::CommandId;
use postio_model::listing::MailStore as _;
use postio_model::{EmailAddress, MessageId};
use postio_ui::focus_dialog;

use crate::Session;
use crate::focus_list::{FocusRowActionFfi, LabelPillFfi, MarkerLineFfi};

/// One person in the sender block: their name when they gave one, and the
/// address, which is drawn in mono.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusPersonFfi {
    /// The display name, when there is one.
    pub name: Option<String>,
    /// The bare address.
    pub address: String,
}

/// One row of the sender block: From, To or Cc.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusFieldFfi {
    /// The field's name, for the 44pt label column: "From".
    pub field: String,
    /// The first few people in it.
    pub people: Vec<FocusPersonFfi>,
    /// How many more there are, said: "and 4 others".
    pub more: Option<String>,
    /// Everyone in it, for the tooltip.
    pub all: String,
}

/// One button of the action row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusVerbFfi {
    /// The registry command it runs.
    pub command: String,
    /// The words on it.
    pub label: String,
    /// Whether it moves into More in a window narrower than the fold.
    pub folds: bool,
}

/// The line above the subject in a conversation: "Latest of 6 in this
/// thread", and the keys that step through it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusThreadChipFfi {
    /// The sentence.
    pub text: String,
    /// `[` and "earlier message", when there is one to step to.
    pub earlier: Option<FocusRowActionFfi>,
    /// `]` and "later message", when there is one.
    pub later: Option<FocusRowActionFfi>,
}

/// An attachment chip under the body.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusAttachmentFfi {
    /// The part, as `save_part` and `open_part` name it.
    pub id: i64,
    /// Its file name, or its type when the sender gave none.
    pub name: String,
    /// Its size, in the words the reader uses: "48 KB".
    pub size: String,
}

/// Everything the message window draws around the body.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusMessageViewFfi {
    /// The message shown.
    pub message: i64,
    /// Its subject, or "(no subject)".
    pub subject: String,
    /// "Message 5 of 60 · thread of 6", with the account when several are
    /// enabled.
    pub position: String,
    /// The thread chip, for a conversation of more than one message.
    pub thread: Option<FocusThreadChipFfi>,
    /// The message `[` shows in this window, if any.
    pub earlier: Option<i64>,
    /// The message `]` shows, if any.
    pub later: Option<i64>,
    /// The conversation's labels.
    pub labels: Vec<LabelPillFfi>,
    /// "+ Label", and the command it runs.
    pub add_label: FocusRowActionFfi,
    /// The sender block's rows: From, then To and Cc when anyone is in them.
    pub fields: Vec<FocusFieldFfi>,
    /// When it was sent: "Today, 15:22".
    pub date: String,
    /// The marker's card, for the message the marker is on.
    pub marker: Option<MarkerLineFfi>,
    /// The card's Dismiss, and the command it runs.
    pub dismiss: FocusRowActionFfi,
    /// The action row, in order: screen 04's verbs, or for a draft on its
    /// way or stopped, the verbs that settle it.
    pub actions: Vec<FocusVerbFfi>,
    /// More, when the row has verbs that fold into it.
    pub more: Option<FocusRowActionFfi>,
    /// The attachment chips.
    pub attachments: Vec<FocusAttachmentFfi>,
}

#[uniffi::export]
impl Session {
    /// What the message window draws around `message`'s body, shown from
    /// row `index` of `total` in Focus's list. `message` is the row's own
    /// message, or one of its conversation's that `[` or `]` stepped to.
    ///
    /// A store read, so never on the main actor.
    pub fn focus_message_view(&self, message: i64, index: u32, total: u32) -> FocusMessageViewFfi {
        // The marker is the list row's, read with its page: the card is
        // the marked message's, so a message `[` stepped to has none. Found
        // by the message, not by `index`, which counts messages where the
        // list also draws digest rows.
        let row = self
            .focus_driver()
            .row_of(message)
            .filter(|row| row.id == message);
        crate::session::blocking(self.focus_message_answers(message, index, total, row))
    }
}

impl Session {
    async fn focus_message_answers(
        &self,
        message: i64,
        index: u32,
        total: u32,
        row: Option<crate::FocusRowFfi>,
    ) -> FocusMessageViewFfi {
        let id = MessageId::new(message);
        let client = self.client();
        let reading = match &client {
            Some(client) => client
                .readings(vec![id], self.is_offline())
                .await
                .ok()
                .and_then(|mut readings| readings.pop()),
            None => None,
        };
        let summary = match &client {
            Some(client) => client
                .message_rows(vec![id])
                .await
                .ok()
                .and_then(|rows| rows.into_iter().next()),
            None => None,
        };
        let thread = summary.as_ref().and_then(|summary| summary.thread);
        let conversation: Vec<i64> = match (&client, thread) {
            (Some(client), Some(thread)) => client
                .conversation(thread)
                .await
                .map(|rows| rows.iter().map(|row| row.id.get()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let labels = match (&client, thread) {
            (Some(client), Some(thread)) => client
                .thread_labels(vec![thread])
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(_, label)| LabelPillFfi {
                    name: label.name,
                    color: label.color,
                })
                .collect(),
            _ => Vec::new(),
        };
        let account = match (&client, reading.as_ref().and_then(|r| r.row.as_deref())) {
            (Some(client), Some(row)) => {
                let accounts = client.accounts().await.unwrap_or_default();
                let enabled: Vec<_> = accounts.iter().filter(|account| account.enabled).collect();
                (enabled.len() > 1)
                    .then(|| {
                        enabled
                            .iter()
                            .find(|account| account.id == row.account_id)
                            .map(|account| account.address.address.clone())
                    })
                    .flatten()
            }
            _ => None,
        };

        // Where the message stands in its conversation. Opened from its row
        // it is the conversation's latest, as GTK's dialog says it, whatever
        // was filed elsewhere since; stepped to, it is where it is.
        let messages = summary
            .as_ref()
            .map_or(1, |summary| summary.thread_count.max(1));
        let at = conversation
            .iter()
            .position(|member| *member == message)
            .unwrap_or(conversation.len().saturating_sub(1));
        let latest = row.is_some() || conversation.is_empty() || at + 1 == conversation.len();
        let step = |by: isize| {
            focus_dialog::step_thread(at, conversation.len(), by).map(|next| conversation[next])
        };
        let send_state = reading.as_ref().and_then(|reading| reading.send_state);
        let position = focus_dialog::with_account(
            &focus_dialog::position_line(
                index as usize,
                total as usize,
                messages,
                at,
                latest,
                send_state,
            ),
            account.as_deref(),
        );
        let thread_chip =
            focus_dialog::thread_chip_at(messages, at, latest).map(|text| FocusThreadChipFfi {
                text,
                earlier: (latest || at > 0)
                    .then(|| action(CommandId::PrevInConversation, focus_dialog::EARLIER_MESSAGE)),
                later: (!latest)
                    .then(|| action(CommandId::NextInConversation, focus_dialog::LATER_MESSAGE)),
            });

        let envelope = reading.as_ref().and_then(|reading| reading.row.as_deref());
        let subject = postio_ui::reader::header::subject_text(
            envelope
                .and_then(|row| row.subject.as_deref())
                .or_else(|| summary.as_ref().and_then(|s| s.subject.as_deref())),
        );
        let mut fields = Vec::new();
        let mut date = String::new();
        let mut attachments = Vec::new();
        if let Some(envelope) = envelope {
            if let Some(field) = field(
                focus_dialog::FIELD_FROM,
                &envelope.from[..envelope.from.len().min(1)],
            ) {
                fields.push(field);
            }
            fields.extend(field(focus_dialog::FIELD_TO, &envelope.to));
            fields.extend(field(focus_dialog::FIELD_CC, &envelope.cc));
            date = postio_ui::focus_row::message_date(
                envelope.date.unwrap_or(envelope.received_at),
                postio_ui::clock::now(),
            );
            let root = postio_ui::reader::parts::root_type(
                envelope.content_type.as_deref(),
                &envelope.body,
                &envelope.attachments,
            );
            attachments = postio_ui::reader::parts::tree(&root, &envelope.attachments)
                .into_iter()
                .filter(postio_ui::reader::parts::Node::is_leaf)
                // As the reader's chips (`postio_widgets::reader::chips`):
                // the body's own text parts are already on screen.
                .filter(|node| !(node.mime.starts_with("text/") && node.filename.is_none()))
                .filter_map(|node| {
                    Some(FocusAttachmentFfi {
                        id: node.attachment?.get(),
                        name: node.label().to_owned(),
                        size: postio_ui::format::human_size(node.size),
                    })
                })
                .collect();
        }

        let (actions, more) = match focus_dialog::send_verbs(send_state) {
            Some(verbs) => (
                focus_dialog::SEND_TOOLBAR
                    .iter()
                    .filter(|verb| verbs.contains(&verb.command))
                    .map(|verb| FocusVerbFfi {
                        command: verb.command.to_string(),
                        label: verb.label.to_owned(),
                        folds: false,
                    })
                    .collect(),
                None,
            ),
            None => (
                focus_dialog::OPEN_TOOLBAR
                    .iter()
                    .filter(|verb| verb.command != CommandId::MoreActions)
                    .map(|verb| FocusVerbFfi {
                        command: verb.command.to_string(),
                        label: verb.label.to_owned(),
                        folds: focus_dialog::FOLDED.contains(&verb.command),
                    })
                    .collect(),
                focus_dialog::OPEN_TOOLBAR
                    .iter()
                    .find(|verb| verb.command == CommandId::MoreActions)
                    .map(|verb| action(verb.command, verb.label)),
            ),
        };

        FocusMessageViewFfi {
            message,
            subject,
            position,
            thread: thread_chip,
            earlier: step(-1),
            later: step(1),
            labels,
            add_label: action(CommandId::AddLabel, focus_dialog::ADD_LABEL),
            fields,
            date,
            marker: row.and_then(|row| row.marker),
            dismiss: action(CommandId::DismissMarker, focus_dialog::DISMISS),
            actions,
            more,
            attachments,
        }
    }
}

/// A command and the words on its button.
fn action(command: CommandId, label: &str) -> FocusRowActionFfi {
    FocusRowActionFfi {
        command: command.to_string(),
        label: label.to_owned(),
    }
}

/// A row of the sender block, or `None` when nobody is in it: a message with
/// no Cc has no Cc line. The first few people, then how many more, as GTK's
/// card draws them (`open_header.rs`).
fn field(name: &str, addresses: &[EmailAddress]) -> Option<FocusFieldFfi> {
    use postio_ui::reader::header::{RECIPIENTS_SHOWN, address_list, others};
    if addresses.is_empty() {
        return None;
    }
    let people = addresses
        .iter()
        .take(RECIPIENTS_SHOWN)
        .map(|address| FocusPersonFfi {
            name: address
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            address: address.address.clone(),
        })
        .collect();
    Some(FocusFieldFfi {
        field: name.to_owned(),
        people,
        more: (addresses.len() > RECIPIENTS_SHOWN)
            .then(|| others(addresses.len() - RECIPIENTS_SHOWN)),
        all: address_list(addresses),
    })
}
