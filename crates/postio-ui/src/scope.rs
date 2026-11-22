//! What a verb reaches from where the focus is: one table, so no frontend can disagree (spec 013
//! D2).

use crate::hints;
use postio_core::{CommandId, Keymap};
use postio_model::ids::{MessageId, ThreadId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFocus {
    Row {
        thread: ThreadId,
        latest: MessageId,
        expanded: bool,
    },
    Child {
        thread: ThreadId,
        message: MessageId,
    },
    Window {
        thread: ThreadId,
        message: MessageId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Thread(ThreadId),
    Message(MessageId),
    /// The verb does not apply here: not offered, and a key does nothing.
    Unavailable,
}

/// A command outside the table is thread-wide; `Unavailable` is only for the two "this message
/// alone" verbs on a row.
pub fn target(focus: ScopeFocus, action: CommandId) -> Scope {
    let (thread, row_message, child_message) = match focus {
        ScopeFocus::Row { thread, latest, .. } => (thread, Some(latest), None),
        ScopeFocus::Child { thread, message } | ScopeFocus::Window { thread, message } => {
            (thread, None, Some(message))
        }
    };
    let here = row_message.or(child_message);
    match action {
        CommandId::Reply | CommandId::ReplyAll | CommandId::Forward | CommandId::Flag => {
            here.map_or(Scope::Unavailable, Scope::Message)
        }
        CommandId::ToggleRead => child_message.map_or(Scope::Thread(thread), Scope::Message),
        CommandId::MoveThisMessage | CommandId::DeleteThisMessage => {
            child_message.map_or(Scope::Unavailable, Scope::Message)
        }
        // `ReplyToLatest` is the row's latest on a row and the thread's latest from a child or the
        // window.
        _ => Scope::Thread(thread),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FooterView {
    pub title: String,
    pub keys: Vec<(String, String)>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterAbout<'a> {
    pub subject: &'a str,
    pub count: usize,
    pub index: usize,
    pub sender: &'a str,
}

/// A row standing for one message has no thread: its title is the subject, its keys are plain, and
/// there is no note.
/// Every keycap is the keymap's, so a rebinding shows.
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
        ScopeFocus::Row { expanded, .. } => (
            format!("{} \u{b7} {} messages", about.subject, about.count),
            "reply to latest",
            "forward latest",
            "archive thread",
            Some(if expanded {
                (CommandId::CollapseThread, "collapse")
            } else {
                (CommandId::ExpandThread, "expand")
            }),
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

pub fn lone(subject: &str, keymap: &Keymap) -> FooterView {
    let keys = [
        (CommandId::Reply, "reply"),
        (CommandId::Forward, "forward"),
        (CommandId::Archive, "archive"),
    ]
    .into_iter()
    .filter_map(|(command, label)| hints::hint(keymap, command, label))
    .map(|hint| (hints::short(&hint.key), hint.label))
    .collect();
    FooterView {
        title: subject.to_owned(),
        keys,
        note: None,
    }
}

/// A digest is no message, so no other row's keys may stay up over it (#2203).
pub fn digest_footer(rule: &str, count: usize, keymap: &Keymap) -> FooterView {
    let size = if count == 1 {
        "1 message".to_owned()
    } else {
        format!("{count} messages")
    };
    let keys = [
        hints::pair(
            keymap,
            CommandId::NextMessage,
            CommandId::PrevMessage,
            "move",
        ),
        hints::hint(keymap, CommandId::OpenMessage, "open"),
        hints::hint(keymap, CommandId::Archive, "archive digest"),
    ]
    .into_iter()
    .flatten()
    .map(|hint| (hints::short(&hint.key), hint.label))
    .collect();
    FooterView {
        title: format!("{rule} \u{b7} {size}"),
        keys,
        note: None,
    }
}

/// Escape goes back to the results, so the footer says so rather than "back to inbox" (#2193).
pub fn over_results(mut view: FooterView, keymap: &Keymap) -> FooterView {
    if let Some(hint) = hints::hint(keymap, CommandId::Back, "back to results") {
        view.keys.push((hints::short(&hint.key), hint.label));
    }
    view
}

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
    format!("{} act on the thread", keys.join(", "))
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
            expanded: false,
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
    fn an_expanded_row_offers_collapse_and_a_collapsed_one_expand() {
        let about = FooterAbout {
            subject: "Harbor",
            count: 6,
            index: 0,
            sender: "",
        };
        let keymap = Keymap::defaults();
        let keys = |expanded| {
            let focus = ScopeFocus::Row {
                thread: T,
                latest: LATEST,
                expanded,
            };
            footer(focus, &about, keymap)
                .keys
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>()
        };
        assert!(
            keys(false).contains(&"expand".to_owned()),
            "{:?}",
            keys(false)
        );
        assert!(!keys(false).contains(&"collapse".to_owned()));
        assert!(
            keys(true).contains(&"collapse".to_owned()),
            "{:?}",
            keys(true)
        );
        assert!(!keys(true).contains(&"expand".to_owned()));
    }

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
            assert!(note.ends_with(" act on the thread"), "{note}");
            assert!(
                note.chars().count() <= 36,
                "the note shares a line with six keys and must fit beside them: {note}"
            );
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

    #[test]
    fn a_digest_row_names_its_digest_and_offers_only_what_acts_on_it() {
        let view = digest_footer("Newsletters", 6, Keymap::defaults());
        assert_eq!(view.title, "Newsletters \u{b7} 6 messages");
        assert_eq!(labels(&view), ["move", "open", "archive digest"]);
        assert_eq!(view.note, None);
        let one = digest_footer("Receipts", 1, Keymap::defaults());
        assert_eq!(one.title, "Receipts \u{b7} 1 message");
    }
}
