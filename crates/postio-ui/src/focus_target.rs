//! Where a Focus verb goes and what the verbs say, with no toolkit in it.
//!
//! The selection, the cursor's row and the message open over the list are a
//! frontend's own; which messages a verb reaches from them, how a command id
//! becomes a host command, and the sentences the verbs answer with are one
//! policy for the desktop and the terminal alike.

use std::collections::HashMap;

use postio_core::state::Selection;
use postio_core::{Command, CommandId, MessageTarget};
use postio_model::EmailAddress;
use postio_model::ids::{DraftId, MessageId, ThreadId};

use crate::focus_list::FocusRow;

/// How long a toast stays before it goes, in seconds: long enough to read the
/// sentence and reach for Undo, far short of the undo stack's own expiry, so
/// Undo keeps working once the toast has gone.
pub const TOAST_SECONDS: u32 = 8;

/// Said when a send verb names a message that is no draft on its way.
pub const NOT_BEING_SENT: &str = "That message is not one being sent";

/// Said when a reply names a message still on its way out, or a draft: it
/// has no other party yet, and answering it would be replying to oneself.
pub const NO_REPLY_TO_OUTGOING: &str = "An outgoing message cannot be replied to";

/// Whether a reply to a message in `state` is refused: a draft being written
/// or on its way, or stopped. A message that was sent is mail like any other
/// -- replying to your own sent message answers its recipients.
pub fn refuses_reply(state: Option<postio_model::DraftState>) -> bool {
    state.is_some_and(|state| state != postio_model::DraftState::Sent)
}

/// Said when `d` on a digest finds its rule gone from the configuration.
pub const RULE_MISSING: &str = "That digest's rule is no longer in config.toml";

/// Said when a message with no links or attachments is asked to open one.
pub const NOTHING_TO_OPEN: &str = "This message has no links or attachments to open";

/// Said when a search is saved with no configuration file to save it to.
pub const NO_CONFIG_TO_SAVE: &str = "There is no config.toml to save the search to";

/// Said when writing a saved search to the configuration failed.
pub const SEARCH_NOT_WRITTEN: &str = "Focus could not write the search to config.toml";

/// What stopping a sender's digesting does, under its title.
pub const STOP_DIGESTING_BODY: &str = "Their mail comes to the inbox again, and what the digest holds from them now comes back with it.";

/// The prompt for `D` in a digest.
pub fn stop_digesting_title(sender: &str) -> String {
    format!("Stop digesting {sender}?")
}

/// What `1`-`4` says when no saved search sits at `index` (zero-based).
pub fn no_saved_search(index: usize) -> String {
    format!("No saved search {} is pinned", index + 1)
}

/// What `U` says once the host answers: the list left, or why not.
pub fn unsubscribed(list: &str) -> String {
    format!("Unsubscribed from {list}")
}

/// What saving a digest rule says.
pub fn rule_saved(name: &str) -> String {
    format!("Digest rule \u{201c}{name}\u{201d} saved")
}

/// What a saved search says.
pub fn search_saved(query: &str) -> String {
    format!("Saved \u{201c}{query}\u{201d}")
}

/// The prompt for removing a digest rule.
pub fn remove_rule_title(name: &str) -> String {
    format!("Remove \u{201c}{name}\u{201d}?")
}

/// What removing a digest rule says once it has released its mail.
pub fn rule_removed(name: &str, released: u32) -> String {
    let messages = if released == 1 { "message" } else { "messages" };
    format!("Removed \u{201c}{name}\u{201d} \u{b7} {released} {messages} back in the inbox")
}

/// What toggling where messages open says: `beside` is whether the window is
/// wide enough to have the pane now.
pub fn reading_placement(pane_wanted: bool, beside: bool) -> &'static str {
    match (pane_wanted, beside) {
        (true, true) => "Messages open beside the list",
        (true, false) => "Messages open beside the list once the window is wider",
        (false, _) => "Messages open over the list",
    }
}

/// Where a verb on mail goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aim {
    /// Every message the view shows but these: a predicate the host
    /// resolves over the inboxes, never a list of what is on screen. The
    /// frontend tells the host the selection first.
    Everything {
        /// Rows taken back out of the selection.
        except: Vec<MessageId>,
    },
    /// These targets, in the order to send them; none when the verb has
    /// nothing to aim at.
    Targets(Vec<MessageTarget>),
}

/// Where a verb goes: the selection when there is one, the cursor's row
/// otherwise. A row folded from several accounts is every copy
/// ([`MessageTarget::Threads`]); a message in no conversation is itself.
/// `reach` is what each picked row reaches, remembered when it was picked.
pub fn aim(
    selection: &Selection,
    reach: &HashMap<MessageId, Vec<ThreadId>>,
    cursor: Option<&FocusRow>,
) -> Aim {
    match selection {
        Selection::Everything { except } => Aim::Everything {
            except: except.clone(),
        },
        Selection::These(picked) if !picked.is_empty() => {
            let mut threads = Vec::new();
            let mut lone = Vec::new();
            for message in picked {
                match reach.get(message) {
                    Some(theirs) if !theirs.is_empty() => threads.extend(theirs.iter().copied()),
                    _ => lone.push(*message),
                }
            }
            let mut aims = Vec::new();
            if !threads.is_empty() {
                aims.push(MessageTarget::Threads(threads));
            }
            if !lone.is_empty() {
                aims.push(MessageTarget::Messages(lone));
            }
            Aim::Targets(aims)
        }
        Selection::These(_) => Aim::Targets(match cursor {
            // A digest stands for its delivery, which its own verbs name; a
            // message verb has nothing to aim at there.
            Some(FocusRow::Digest(_)) | None => Vec::new(),
            Some(row) if !row.threads().is_empty() => vec![MessageTarget::Threads(row.threads())],
            Some(row) => vec![MessageTarget::Messages(vec![row.id()])],
        }),
    }
}

/// The one message a reply or an answer is about: the message open over the
/// list when one is (`open` is `Some` then, holding what it shows), the
/// cursor's row otherwise.
pub fn aimed_message(
    open: Option<Option<MessageId>>,
    cursor: Option<&FocusRow>,
) -> Option<MessageId> {
    match open {
        Some(shown) => shown,
        None => cursor.map(FocusRow::id),
    }
}

/// The one message "Digest mail like this" would check other mail against:
/// the cursor's, when nothing beyond it is selected and the person has a
/// model for it (`enabled`).
pub fn like_this_message(
    selection: &Selection,
    enabled: bool,
    cursor: Option<&FocusRow>,
) -> Option<MessageId> {
    let single = match selection {
        Selection::These(picked) => picked.len() <= 1,
        Selection::Everything { .. } => false,
    };
    if !single || !enabled {
        return None;
    }
    Some(cursor?.as_conversation()?.summary.representative.id)
}

/// The rows a verb about senders is about: those of `resident` the
/// selection names, or the cursor's when it names none.
pub fn aimed_rows(
    selection: &Selection,
    cursor: Option<FocusRow>,
    resident: impl IntoIterator<Item = FocusRow>,
) -> Vec<FocusRow> {
    let picked: &[MessageId] = match selection {
        Selection::These(picked) => picked,
        Selection::Everything { .. } => &[],
    };
    if picked.is_empty() {
        return cursor.into_iter().collect();
    }
    resident
        .into_iter()
        .filter(|row| picked.contains(&row.id()))
        .collect()
}

/// The senders of `rows`, once each.
pub fn senders(rows: &[FocusRow]) -> Vec<EmailAddress> {
    let mut senders: Vec<EmailAddress> = Vec::new();
    for row in rows {
        let Some(from) = row
            .as_conversation()
            .and_then(|row| row.summary.representative.from.clone())
        else {
            continue;
        };
        if !senders.iter().any(|known| known.same_address(&from)) {
            senders.push(from);
        }
    }
    senders
}

/// What a picker names as its target: the cursor's conversation, or how many
/// are selected. Empty when there is nothing to name.
pub fn picker_target(selection: &Selection, cursor: Option<&FocusRow>) -> String {
    let selected = match selection {
        Selection::These(picked) => picked.len(),
        Selection::Everything { .. } => return "Every conversation".to_owned(),
    };
    let Some(row) = cursor.and_then(FocusRow::as_conversation) else {
        return String::new();
    };
    let representative = &row.summary.representative;
    let sender = representative
        .from
        .as_ref()
        .map(|from| from.display().to_owned())
        .unwrap_or_default();
    let subject = row
        .summary
        .subject
        .clone()
        .or_else(|| representative.subject.clone())
        .unwrap_or_default();
    crate::pickers::target(selected.max(1), &sender, &subject)
}

/// How a command id reaches the host from the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Dispatch {
    /// A verb on mail: sent once per aim, then the selection lets go.
    OnMail(Command),
    /// A verb that aims at nothing: sent as it is.
    Plain(Command),
}

/// The host command a list verb means, or `None` when the verb is the
/// frontend's own (opening, picking, going somewhere). `A` is `a` here: a
/// Focus row is a whole conversation.
pub fn dispatch(id: CommandId) -> Option<Dispatch> {
    match id {
        CommandId::Archive | CommandId::ArchiveThread => {
            Some(Dispatch::OnMail(Command::default_for(CommandId::Archive)))
        }
        CommandId::Delete | CommandId::ToggleRead | CommandId::Flag | CommandId::Unsnooze => {
            Some(Dispatch::OnMail(Command::default_for(id)))
        }
        CommandId::Undo => Some(Dispatch::Plain(Command::Undo)),
        CommandId::Refresh => Some(Dispatch::Plain(Command::Refresh)),
        _ => None,
    }
}

/// The command that settles the send of `draft`, for the send verb `id`:
/// cancel it, retry it, or mark it sent.
pub fn settle_command(id: CommandId, draft: Option<DraftId>) -> Command {
    match id {
        CommandId::CancelSend => Command::CancelSend { draft },
        CommandId::RetrySend => Command::RetrySend { draft },
        _ => Command::MarkSent { draft },
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use postio_model::listing::{MessageSummary, ThreadSummary};

    use super::*;

    #[test]
    fn only_a_message_not_yet_sent_refuses_a_reply() {
        use postio_model::DraftState;
        assert!(!refuses_reply(None));
        assert!(!refuses_reply(Some(DraftState::Sent)));
        for state in [
            DraftState::Editing,
            DraftState::Queued,
            DraftState::Sending,
            DraftState::Failed,
            DraftState::Unconfirmed,
        ] {
            assert!(refuses_reply(Some(state)), "{state:?} answered a reply");
        }
    }

    fn row(id: i64, thread: Option<i64>, copies: &[i64]) -> FocusRow {
        FocusRow::conversation(ThreadSummary {
            id: thread.map(ThreadId::new),
            representative: MessageSummary {
                id: MessageId::new(id),
                thread: thread.map(ThreadId::new),
                from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
                subject: Some("Plans".into()),
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
            subject: Some("Plans".into()),
            participants: Vec::new(),
            message_count: 1,
            unread_count: 1,
            flagged: false,
            has_attachments: false,
            last_at: Utc::now(),
            marker: None,
            copies: copies.iter().map(|id| ThreadId::new(*id)).collect(),
        })
    }

    #[test]
    fn with_nothing_picked_a_verb_aims_at_the_cursors_whole_conversation() {
        let folded = row(1, Some(3), &[9]);
        assert_eq!(
            aim(&Selection::default(), &HashMap::new(), Some(&folded)),
            Aim::Targets(vec![MessageTarget::Threads(vec![
                ThreadId::new(3),
                ThreadId::new(9)
            ])])
        );
        assert_eq!(
            aim(
                &Selection::default(),
                &HashMap::new(),
                Some(&row(2, None, &[]))
            ),
            Aim::Targets(vec![MessageTarget::Messages(vec![MessageId::new(2)])])
        );
        assert_eq!(
            aim(&Selection::default(), &HashMap::new(), None),
            Aim::Targets(Vec::new())
        );
    }

    #[test]
    fn a_selection_aims_by_what_each_picked_row_reaches() {
        let reach = HashMap::from([(MessageId::new(1), vec![ThreadId::new(3)])]);
        let picked = Selection::These(vec![MessageId::new(1), MessageId::new(2)]);
        assert_eq!(
            aim(&picked, &reach, None),
            Aim::Targets(vec![
                MessageTarget::Threads(vec![ThreadId::new(3)]),
                MessageTarget::Messages(vec![MessageId::new(2)]),
            ])
        );
        assert_eq!(
            aim(
                &Selection::Everything {
                    except: vec![MessageId::new(4)]
                },
                &reach,
                None
            ),
            Aim::Everything {
                except: vec![MessageId::new(4)]
            }
        );
    }

    #[test]
    fn an_open_message_is_the_one_aimed_at_even_when_it_shows_nothing_yet() {
        let cursor = row(5, Some(5), &[]);
        assert_eq!(
            aimed_message(Some(Some(MessageId::new(7))), Some(&cursor)),
            Some(MessageId::new(7))
        );
        assert_eq!(aimed_message(Some(None), Some(&cursor)), None);
        assert_eq!(aimed_message(None, Some(&cursor)), Some(MessageId::new(5)));
    }

    #[test]
    fn like_this_needs_one_row_and_a_model() {
        let cursor = row(5, Some(5), &[]);
        let one = Selection::default();
        assert_eq!(
            like_this_message(&one, true, Some(&cursor)),
            Some(MessageId::new(5))
        );
        assert_eq!(like_this_message(&one, false, Some(&cursor)), None);
        let two = Selection::These(vec![MessageId::new(1), MessageId::new(2)]);
        assert_eq!(like_this_message(&two, true, Some(&cursor)), None);
        assert_eq!(
            like_this_message(
                &Selection::Everything { except: Vec::new() },
                true,
                Some(&cursor)
            ),
            None
        );
    }

    #[test]
    fn senders_are_each_named_once() {
        let rows = aimed_rows(
            &Selection::These(vec![MessageId::new(1), MessageId::new(2)]),
            None,
            vec![
                row(1, Some(1), &[]),
                row(2, Some(2), &[]),
                row(3, Some(3), &[]),
            ],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(senders(&rows).len(), 1);
        let cursor = row(9, Some(9), &[]);
        assert_eq!(
            aimed_rows(&Selection::default(), Some(cursor.clone()), Vec::new()),
            vec![cursor]
        );
    }

    #[test]
    fn a_picker_names_its_target() {
        let cursor = row(1, Some(1), &[]);
        assert_eq!(
            picker_target(&Selection::default(), Some(&cursor)),
            "Ada \u{b7} Plans"
        );
        assert_eq!(
            picker_target(
                &Selection::These(vec![MessageId::new(1), MessageId::new(2)]),
                Some(&cursor)
            ),
            "2 conversations"
        );
        assert_eq!(
            picker_target(&Selection::Everything { except: Vec::new() }, None),
            "Every conversation"
        );
        assert_eq!(picker_target(&Selection::default(), None), "");
    }

    #[test]
    fn the_list_verbs_the_host_answers_are_a_table() {
        assert_eq!(
            dispatch(CommandId::ArchiveThread),
            Some(Dispatch::OnMail(Command::default_for(CommandId::Archive)))
        );
        assert_eq!(
            dispatch(CommandId::Undo),
            Some(Dispatch::Plain(Command::Undo))
        );
        assert_eq!(dispatch(CommandId::OpenMessage), None);
        assert_eq!(
            settle_command(CommandId::RetrySend, Some(DraftId::new(4))),
            Command::RetrySend {
                draft: Some(DraftId::new(4))
            }
        );
    }

    #[test]
    fn the_sentences_read_as_the_window_says_them() {
        assert_eq!(no_saved_search(1), "No saved search 2 is pinned");
        assert_eq!(
            reading_placement(true, false),
            "Messages open beside the list once the window is wider"
        );
        assert_eq!(
            rule_removed("News", 1),
            "Removed \u{201c}News\u{201d} \u{b7} 1 message back in the inbox"
        );
    }
}
