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

use postio_core::CommandId;
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
        // T023: `MoveThisMessage` and `DeleteThisMessage` go here: this
        // message on a child or in the window, `Unavailable` on a row.
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

/// The scope's title under the list and in the window: the thread's subject
/// and size on a row, the message's place and sender below it.
///
/// `index` counts from zero, oldest first. Plain inputs, so both frontends
/// and the tests say the same sentence; the keys beside it are the footer
/// view's, later.
pub fn footer(
    focus: ScopeFocus,
    subject: &str,
    count: usize,
    index: usize,
    sender: &str,
) -> String {
    match focus {
        ScopeFocus::Row { .. } => {
            let messages = if count == 1 { "message" } else { "messages" };
            format!("{subject} \u{b7} {count} {messages}")
        }
        ScopeFocus::Child { .. } | ScopeFocus::Window { .. } => {
            format!("Message {} of {count} \u{b7} {sender}", index + 1)
        }
    }
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
    fn a_verb_the_table_does_not_name_stays_thread_wide() {
        for focus in [row(), child(), window()] {
            for id in [CommandId::ArchiveThread, CommandId::Unsnooze] {
                assert_eq!(target(focus, id), Scope::Thread(T), "{focus:?} {id:?}");
            }
        }
    }

    #[test]
    fn the_footer_names_the_thread_on_a_row() {
        assert_eq!(
            footer(row(), "Harbor API draft v3", 6, 5, "Lena Ortiz"),
            "Harbor API draft v3 \u{b7} 6 messages"
        );
        assert_eq!(
            footer(row(), "Lunch", 1, 0, "Ada"),
            "Lunch \u{b7} 1 message"
        );
    }

    #[test]
    fn the_footer_places_the_message_below_a_row() {
        let said = "Message 4 of 6 \u{b7} Ben Adeyemi";
        assert_eq!(footer(child(), "Harbor", 6, 3, "Ben Adeyemi"), said);
        assert_eq!(footer(window(), "Harbor", 6, 3, "Ben Adeyemi"), said);
    }
}
