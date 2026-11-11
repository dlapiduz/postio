//! What a verb reaches from where the focus is: one function, one table.
//!
//! A thread in Focus is a row that opens into its messages, so "reply" on the
//! row and "reply" on message 4 of 6 cannot mean the same thing. Spec 013's
//! D2 settles it by the design's table, and this module is the only place
//! the table lives: the list, the message window and the thread menu all ask
//! [`target`], so a cell cannot be right in one frontend and wrong in
//! another.
//!
//! | Focus | Reply, reply all, forward, flag | Archive, snooze, label, move, remind, mute, delete | Read / unread | Move or delete only this |
//! |---|---|---|---|---|
//! | Collapsed row | latest message | thread | thread | not offered |
//! | Child row, window | this message | thread | this message | this message |

use crate::hints;
use postio_core::{CommandId, Keymap};
use postio_model::ids::{MessageId, ThreadId};

/// Where the focus is, as far as scope cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFocus {
    /// A collapsed thread row; `latest` is the message the row shows.
    Row {
        /// The conversation the row stands for.
        thread: ThreadId,
        /// Its latest message.
        latest: MessageId,
    },
    /// A child row of the expanded thread.
    Child {
        /// The conversation it belongs to.
        thread: ThreadId,
        /// The child's message.
        message: MessageId,
    },
    /// The message window, showing one message of a thread.
    Window {
        /// The conversation it belongs to.
        thread: ThreadId,
        /// The message shown.
        message: MessageId,
    },
}

/// What a verb reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every message of the thread.
    Thread(ThreadId),
    /// This one message.
    Message(MessageId),
    /// The verb does not apply here: not offered, and a key does nothing.
    Unavailable,
}

/// What `action` reaches from `focus`, by spec 013's D2.
///
/// A command outside the table is thread-wide, as every verb was before
/// threads had children: that keeps the verbs the table does not name (a
/// dismissed marker, an unsnooze) aimed as they always were, rather than
/// dropping them as [`Scope::Unavailable`], which means "not offered here"
/// and is only for the two "this message alone" verbs on a row.
pub fn target(focus: ScopeFocus, action: CommandId) -> Scope {
    let (thread, row_message, child_message) = match focus {
        ScopeFocus::Row { thread, latest } => (thread, Some(latest), None),
        ScopeFocus::Child { thread, message } | ScopeFocus::Window { thread, message } => {
            (thread, None, Some(message))
        }
    };
    let here = row_message.or(child_message);
    match action {
        // A message's own: the row's latest, or this message. A flag is a
        // message's, like a reply.
        CommandId::Reply | CommandId::ReplyAll | CommandId::Forward | CommandId::Flag => {
            here.map_or(Scope::Unavailable, Scope::Message)
        }
        // Read state is the thread's on a row, the message's below it.
        CommandId::ToggleRead => child_message.map_or(Scope::Thread(thread), Scope::Message),
        // Move or delete only this: this message on a child or in the
        // window, not offered on a row, which stands for the whole thread.
        CommandId::MoveThisMessage | CommandId::DeleteThisMessage => {
            child_message.map_or(Scope::Unavailable, Scope::Message)
        }
        // T023: `ReplyToLatest` is the row's latest on a row, and the
        // thread's latest from a child or the window, not this message.
        // T023: `MuteThread` is thread-wide and falls to the default arm.
        // T023: forwarding the whole thread is `Scope::Thread` and the same.
        //
        // Archive, snooze, label, move, remind and delete are the thread's
        // everywhere; so is everything the table does not name.
        _ => Scope::Thread(thread),
    }
}

/// What the footer under the list says (spec 013 US2): the scope in words,
/// the keys that matter there, and one sentence on what acts on what.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FooterView {
    /// The scope: "Harbor API draft v3 \u{b7} 6 messages", "Message 4 of 6 \u{b7} Ben Adeyemi".
    pub title: String,
    /// Keycap and label pairs, in the order they are drawn.
    pub keys: Vec<(String, String)>,
    /// The right-hand sentence; none on a row that stands for one message.
    pub note: Option<String>,
}

/// What the footer is about: the thread's subject and size, and the
/// message the focus is on when it is on one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterAbout<'a> {
    /// The thread's subject.
    pub subject: &'a str,
    /// How many messages it holds.
    pub count: usize,
    /// The focused message, from zero, oldest first.
    pub index: usize,
    /// Who sent it: "You" for the person's own.
    pub sender: &'a str,
}

/// The footer under the list, by spec 013's D2: its title says the scope,
/// its keys say what reply, forward and archive reach from here, and the
/// note under a child says which keys act on the thread.
///
/// A row that stands for one message has no thread to speak of: its title
/// is the subject, its keys are plain "reply", "forward" and "archive", and
/// there is no note. Every keycap is the keymap's, so a rebinding shows.
pub fn footer(focus: ScopeFocus, about: &FooterAbout<'_>, keymap: &Keymap) -> FooterView {
    let (title, reply, forward, archive, walk, note) = match focus {
        ScopeFocus::Row { .. } if about.count <= 1 => (
            about.subject.to_owned(),
            "reply",
            "forward",
            "archive",
            Some((CommandId::OpenMessage, "open")),
            None,
        ),
        ScopeFocus::Row { .. } => (
            format!("{} \u{b7} {} messages", about.subject, about.count),
            "reply to latest",
            "forward latest",
            "archive thread",
            Some((CommandId::ExpandThread, "expand")),
            None,
        ),
        ScopeFocus::Child { .. } | ScopeFocus::Window { .. } => (
            format!(
                "Message {} of {} \u{b7} {}",
                about.index + 1,
                about.count,
                about.sender
            ),
            "reply to this",
            "forward this",
            "archive thread",
            matches!(focus, ScopeFocus::Child { .. })
                .then_some((CommandId::CollapseThread, "collapse")),
            Some(thread_note(keymap)),
        ),
    };
    let mut keys = Vec::new();
    let mut push = |hint: Option<hints::Hint>| {
        keys.extend(hint.map(|hint| (hints::short(&hint.key), hint.label)));
    };
    if !matches!(focus, ScopeFocus::Window { .. }) {
        push(hints::pair(
            keymap,
            CommandId::NextMessage,
            CommandId::PrevMessage,
            "move",
        ));
        let open = if matches!(focus, ScopeFocus::Child { .. }) {
            "open here"
        } else {
            "open"
        };
        push(hints::hint(keymap, CommandId::OpenMessage, open));
    }
    push(hints::hint(keymap, CommandId::Reply, reply));
    push(hints::hint(keymap, CommandId::Forward, forward));
    push(hints::hint(keymap, CommandId::Archive, archive));
    if let Some((command, label)) = walk
        && command != CommandId::OpenMessage
    {
        push(hints::hint(keymap, command, label));
    }
    FooterView { title, keys, note }
}

/// "Reply and forward act on this message; e, b, l, v act on the thread",
/// with the keys the keymap binds to archive, snooze, label and move.
fn thread_note(keymap: &Keymap) -> String {
    let keys: Vec<&str> = [
        CommandId::Archive,
        CommandId::Snooze,
        CommandId::AddLabel,
        CommandId::Move,
    ]
    .into_iter()
    .filter_map(|command| keymap.binding(command))
    .collect();
    format!(
        "Reply and forward act on this message; {} act on the thread",
        keys.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: ThreadId = ThreadId::new(7);
    const LATEST: MessageId = MessageId::new(60);
    const THIS: MessageId = MessageId::new(40);

    fn row() -> ScopeFocus {
        ScopeFocus::Row {
            thread: T,
            latest: LATEST,
        }
    }

    fn child() -> ScopeFocus {
        ScopeFocus::Child {
            thread: T,
            message: THIS,
        }
    }

    fn window() -> ScopeFocus {
        ScopeFocus::Window {
            thread: T,
            message: THIS,
        }
    }

    const MESSAGE_SCOPED: [CommandId; 4] = [
        CommandId::Reply,
        CommandId::ReplyAll,
        CommandId::Forward,
        CommandId::Flag,
    ];
    const THREAD_SCOPED: [CommandId; 6] = [
        CommandId::Archive,
        CommandId::Snooze,
        CommandId::AddLabel,
        CommandId::Move,
        CommandId::RemindIfNoReply,
        CommandId::Delete,
    ];

    #[test]
    fn on_a_row_reply_reply_all_forward_and_flag_go_to_the_latest_message() {
        for id in MESSAGE_SCOPED {
            assert_eq!(target(row(), id), Scope::Message(LATEST), "{id:?}");
        }
    }

    #[test]
    fn on_a_child_or_in_the_window_they_go_to_this_message() {
        for id in MESSAGE_SCOPED {
            assert_eq!(target(child(), id), Scope::Message(THIS), "{id:?}");
            assert_eq!(target(window(), id), Scope::Message(THIS), "{id:?}");
        }
    }

    #[test]
    fn archive_snooze_label_move_remind_and_delete_go_to_the_thread_everywhere() {
        for focus in [row(), child(), window()] {
            for id in THREAD_SCOPED {
                assert_eq!(target(focus, id), Scope::Thread(T), "{focus:?} {id:?}");
            }
        }
    }

    #[test]
    fn read_and_unread_is_the_threads_on_a_row_and_the_messages_below_it() {
        assert_eq!(target(row(), CommandId::ToggleRead), Scope::Thread(T));
        assert_eq!(target(child(), CommandId::ToggleRead), Scope::Message(THIS));
        assert_eq!(
            target(window(), CommandId::ToggleRead),
            Scope::Message(THIS)
        );
    }

    #[test]
    fn move_or_delete_only_this_is_the_message_below_a_row_and_not_offered_on_one() {
        for id in [CommandId::MoveThisMessage, CommandId::DeleteThisMessage] {
            assert_eq!(target(row(), id), Scope::Unavailable, "{id:?}");
            assert_eq!(target(child(), id), Scope::Message(THIS), "{id:?}");
            assert_eq!(target(window(), id), Scope::Message(THIS), "{id:?}");
        }
    }

    #[test]
    fn a_verb_the_table_does_not_name_stays_thread_wide() {
        for focus in [row(), child(), window()] {
            for id in [CommandId::Archive, CommandId::Unsnooze] {
                assert_eq!(target(focus, id), Scope::Thread(T), "{focus:?} {id:?}");
            }
        }
    }

    fn about(count: usize, index: usize) -> FooterAbout<'static> {
        FooterAbout {
            subject: "Harbor API draft v3",
            count,
            index,
            sender: "Ben Adeyemi",
        }
    }

    fn labels(view: &FooterView) -> Vec<&str> {
        view.keys.iter().map(|(_, label)| label.as_str()).collect()
    }

    #[test]
    fn the_footer_names_the_thread_on_a_row_and_what_its_keys_reach() {
        let view = footer(row(), &about(6, 5), Keymap::defaults());
        assert_eq!(view.title, "Harbor API draft v3 \u{b7} 6 messages");
        let said = labels(&view);
        for want in [
            "reply to latest",
            "forward latest",
            "archive thread",
            "expand",
        ] {
            assert!(said.contains(&want), "{want} in {said:?}");
        }
        assert_eq!(view.note, None);
    }

    #[test]
    fn the_footer_places_the_message_below_a_row_and_says_what_acts_on_what() {
        for focus in [child(), window()] {
            let view = footer(focus, &about(6, 3), Keymap::defaults());
            assert_eq!(view.title, "Message 4 of 6 \u{b7} Ben Adeyemi");
            let said = labels(&view);
            for want in ["reply to this", "forward this", "archive thread"] {
                assert!(said.contains(&want), "{want} in {said:?}");
            }
            let note = view.note.expect("a note below a row");
            assert!(note.starts_with("Reply and forward act on this message; "));
            assert!(note.ends_with(" act on the thread"));
        }
    }

    #[test]
    fn a_row_of_one_message_has_no_thread_wording() {
        let view = footer(row(), &about(1, 0), Keymap::defaults());
        assert_eq!(view.title, "Harbor API draft v3");
        assert_eq!(view.note, None);
        for said in labels(&view) {
            assert!(
                !["thread", "latest", "this", "expand"]
                    .iter()
                    .any(|w| said.contains(w)),
                "{said}"
            );
        }
    }
}
