//! Turns a `CommandId` into the `Command` it means for the current selection and cursor.
//!
//! The rule lives here so GTK, macOS and MCP agree (#589, ADR 0019); frontends supply only row
//! facts.

use postio_model::{AccountId, ListScope, MessageId, ThreadId};

use crate::bridge::EventSink;
use crate::command::{Command, CommandId, MessageTarget};
use crate::state::{Selection, SharedState, ViewScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Not an error: a marked row no longer resident cannot be checked, and guessing it is a
    /// conversation would act on unmarked mail (#468).
    Missing,
    Message,
    Thread(ThreadId),
}

pub trait RowFacts {
    fn row_kind(&self, message: MessageId) -> RowKind;
}

/// Borrowed: built per keystroke from state the frontend already has.
///
/// There is no context field: row kinds already exclude thread drill-ins.
pub struct Aim<'a> {
    /// `None` when a whole-view gesture is meaningless: a thread drill-in, or Snoozed (#493).
    pub scope: Option<ViewScope>,
    pub selection: &'a Selection,
    /// `AppState::resolve` falls back to it when the selection is empty.
    pub cursor: Option<MessageId>,
    pub rows: &'a dyn RowFacts,
}

/// `None` where a whole-view gesture is meaningless: a thread drill-in, Snoozed (#493), or an
/// aggregate view with nothing reachable. A rejection beats a no-op that claims to have acted.
pub fn view_scope(scope: ListScope, reachable: &[AccountId]) -> Option<ViewScope> {
    match scope {
        ListScope::Mailbox(mailbox) => Some(ViewScope::Mailbox(mailbox)),
        ListScope::Flagged(account) => Some(ViewScope::Flagged(account)),
        // `reachable` is required: a frontend that has not decided which accounts its list shows
        // has no whole-view meaning. Empty means no view to be relative to (#811).
        ListScope::Unified if reachable.is_empty() => None,
        ListScope::Unified => Some(ViewScope::Unified {
            accounts: reachable.to_vec(),
        }),
        // Outbox: cancel or discard over everything in flight must not be one key away.
        // Focus's inbox is a predicate over the accounts it could show, like the aggregate.
        ListScope::Focus(postio_model::FocusScope::Inbox) if reachable.is_empty() => None,
        ListScope::Focus(postio_model::FocusScope::Inbox) => Some(ViewScope::Focus {
            accounts: reachable.to_vec(),
        }),
        ListScope::Account(_)
        | ListScope::Snoozed(_)
        | ListScope::Outbox(_)
        | ListScope::Thread(_)
        | ListScope::Focus(
            postio_model::FocusScope::HasAction
            | postio_model::FocusScope::Snoozed
            | postio_model::FocusScope::Flagged,
        ) => None,
    }
}

pub fn command_for(id: CommandId, aim: &Aim<'_>) -> Command {
    refine(Command::default_for(id), aim)
}

/// A verb on a thread row acts on the conversation, not the one message the row is drawn from
/// (ADR 0015 Q3). Only a verb still aimed at the selection is re-aimed, and only over thread rows;
/// a marked set of thread rows becomes `MessageTarget::Threads` (#468).
pub fn refine(command: Command, aim: &Aim<'_>) -> Command {
    if !matches!(command.target(), Some(MessageTarget::Selection)) {
        return command;
    }
    match aim.selection {
        Selection::These(marked) if marked.is_empty() => {
            let Some(cursor) = aim.cursor else {
                return command;
            };
            match aim.rows.row_kind(cursor) {
                RowKind::Thread(thread) => command.with_target(MessageTarget::Thread(thread)),
                RowKind::Message | RowKind::Missing => command,
            }
        }
        Selection::These(marked) => match threads_of(aim.rows, marked) {
            Some(threads) => command.with_target(MessageTarget::Threads(threads)),
            // Not all thread rows (query view, or a folder mid-switch): keep `Selection`, narrower
            // than a conversation but never wrong.
            None => command,
        },
        // Exceptions are message ids; excepting a conversation needs a `MessageSet` variant (#468).
        Selection::Everything { .. } => command,
    }
}

/// All-or-nothing: a mixed target would archive some rows as conversations and some as messages.
fn threads_of(rows: &dyn RowFacts, marked: &[MessageId]) -> Option<Vec<ThreadId>> {
    let mut threads = Vec::with_capacity(marked.len());
    for id in marked {
        match rows.row_kind(*id) {
            RowKind::Thread(thread) => threads.push(thread),
            RowKind::Message | RowKind::Missing => return None,
        }
    }
    threads.sort_unstable();
    threads.dedup();
    Some(threads)
}

/// Pulls the frontend's selection into app state just before a command is sent, not pushed on
/// every `j`. The events go to `quiet`, a sink whose reader was dropped.
pub fn mirror(state: &SharedState, quiet: &EventSink, aim: &Aim<'_>) {
    state.update(quiet, |app| {
        let mut events = Vec::new();
        if let Some(scope) = aim.scope.clone() {
            // Opening a view drops the selection, so this goes first. Scope, not mailbox: a smart
            // folder has no mailbox (#52).
            events.extend(app.open_view(scope));
        }
        match aim.selection {
            Selection::These(messages) => events.extend(app.select(messages.clone(), aim.cursor)),
            // Re-apply exceptions one by one: resolving the predicate to ids is the mailbox-sized
            // read.
            Selection::Everything { except } => {
                events.extend(app.select_all());
                for message in except {
                    events.extend(app.toggle_selection(*message));
                }
                events.extend(app.focus_on(aim.cursor));
            }
        }
        events
    });
}

/// Gestures other consumers answer (composer, window `Esc`) must not reach the bus and draw a
/// "not wired up" rejection.
pub fn is_wired(wired: &[CommandId], command: &Command) -> bool {
    wired.contains(&command.id())
}

#[derive(Clone, Debug)]
pub struct Conformance {
    pub id: CommandId,
    pub selection: Selection,
    pub cursor: Option<MessageId>,
    /// Each frontend fills its own row source from these.
    pub rows: Vec<(MessageId, RowKind)>,
    pub expected: Command,
    pub because: &'static str,
}

/// Each frontend builds its own `Aim` from `rows` and asserts `expected`, so adapters cannot
/// disagree on what they hand `command_for` (#589, #721).
pub fn conformance_cases() -> Vec<Conformance> {
    let thread_row = MessageId::new(1);
    let thread = ThreadId::new(11);
    let message_row = MessageId::new(2);
    let gone = MessageId::new(99);
    let rows = || {
        vec![
            (thread_row, RowKind::Thread(thread)),
            (message_row, RowKind::Message),
        ]
    };
    let nothing_marked = || Selection::These(Vec::new());

    vec![
        Conformance {
            id: CommandId::Archive,
            selection: nothing_marked(),
            cursor: Some(thread_row),
            rows: rows(),
            expected: Command::Archive {
                target: MessageTarget::Thread(thread),
            },
            because: "a verb on a thread row acts on the conversation (ADR 0015 Q3)",
        },
        Conformance {
            id: CommandId::Archive,
            selection: nothing_marked(),
            cursor: Some(message_row),
            rows: rows(),
            expected: Command::Archive {
                target: MessageTarget::Selection,
            },
            because: "a message row is one message, so the verb stays as it was",
        },
        Conformance {
            id: CommandId::Archive,
            selection: nothing_marked(),
            cursor: Some(gone),
            rows: rows(),
            expected: Command::Archive {
                target: MessageTarget::Selection,
            },
            because: "a cursor on a row nobody holds any more must not be \
                      guessed into a conversation (#468)",
        },
        Conformance {
            id: CommandId::Archive,
            selection: Selection::These(vec![message_row]),
            cursor: Some(thread_row),
            rows: rows(),
            expected: Command::Archive {
                target: MessageTarget::Selection,
            },
            because: "something is marked, so the gesture is about the marks \
                      rather than about the row the cursor happens to be on",
        },
        Conformance {
            id: CommandId::Archive,
            selection: Selection::Everything { except: Vec::new() },
            cursor: Some(thread_row),
            rows: rows(),
            expected: Command::Archive {
                target: MessageTarget::Selection,
            },
            because: "a whole-view selection is a predicate; narrowing it to \
                      the cursor's conversation would act on less than was asked",
        },
        Conformance {
            id: CommandId::Flag,
            selection: nothing_marked(),
            cursor: Some(thread_row),
            rows: rows(),
            expected: Command::Flag {
                target: MessageTarget::Thread(thread),
                flagged: None,
            },
            because: "the rule is about the target, not about which verb it is",
        },
        Conformance {
            id: CommandId::Refresh,
            selection: nothing_marked(),
            cursor: Some(thread_row),
            rows: rows(),
            expected: Command::Refresh,
            because: "a command with no message target is handed over untouched",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct FakeRows(BTreeMap<i64, RowKind>);

    impl FakeRows {
        fn threads(ids: &[(i64, i64)]) -> Self {
            FakeRows(
                ids.iter()
                    .map(|(message, thread)| (*message, RowKind::Thread(ThreadId::new(*thread))))
                    .collect(),
            )
        }

        fn messages(ids: &[i64]) -> Self {
            FakeRows(ids.iter().map(|id| (*id, RowKind::Message)).collect())
        }
    }

    impl RowFacts for FakeRows {
        fn row_kind(&self, message: MessageId) -> RowKind {
            self.0
                .get(&message.get())
                .copied()
                .unwrap_or(RowKind::Missing)
        }
    }

    fn message(id: i64) -> MessageId {
        MessageId::new(id)
    }

    #[test]
    fn only_a_folder_and_flagged_are_something_ctrl_a_can_be_relative_to() {
        use postio_model::{AccountId, MailboxId};

        assert_eq!(
            view_scope(ListScope::Mailbox(MailboxId::new(4)), &[]),
            Some(ViewScope::Mailbox(MailboxId::new(4))),
        );
        assert_eq!(
            view_scope(ListScope::Flagged(AccountId::new(1)), &[]),
            Some(ViewScope::Flagged(AccountId::new(1))),
        );
        assert_eq!(
            view_scope(ListScope::Account(AccountId::new(1)), &[]),
            None,
            "nothing needs an account-wide `MessageSet` predicate yet",
        );
        assert_eq!(
            view_scope(ListScope::Snoozed(AccountId::new(1)), &[]),
            None,
            "nothing needs a `MessageSet::Snoozed` predicate yet (#493)",
        );
        assert_eq!(
            view_scope(ListScope::Thread(ThreadId::new(3)), &[]),
            None,
            "`Ctrl+A` inside a conversation is not a gesture",
        );
    }

    #[test]
    fn the_unified_view_is_relative_to_the_accounts_it_could_actually_show() {
        use postio_model::AccountId;

        assert_eq!(
            view_scope(ListScope::Unified, &[AccountId::new(1), AccountId::new(3)]),
            Some(ViewScope::Unified {
                accounts: vec![AccountId::new(1), AccountId::new(3)],
            }),
        );
        assert_eq!(
            view_scope(ListScope::Unified, &[]),
            None,
            "no account is reachable, so there is no view to be relative to",
        );
    }

    #[test]
    fn focus_s_inbox_is_a_scope_of_its_own_over_the_accounts_it_could_show() {
        use postio_model::{AccountId, FocusScope};

        assert_eq!(
            view_scope(ListScope::Focus(FocusScope::Inbox), &[AccountId::new(2)]),
            Some(ViewScope::Focus {
                accounts: vec![AccountId::new(2)],
            }),
        );
        assert_eq!(view_scope(ListScope::Focus(FocusScope::Inbox), &[]), None);
        assert_eq!(
            view_scope(
                ListScope::Focus(FocusScope::HasAction),
                &[AccountId::new(2)]
            ),
            None,
        );
    }

    fn aim<'a>(selection: &'a Selection, cursor: Option<i64>, rows: &'a dyn RowFacts) -> Aim<'a> {
        Aim {
            scope: None,
            selection,
            cursor: cursor.map(message),
            rows,
        }
    }

    #[test]
    fn a_verb_on_the_cursors_thread_row_acts_on_the_conversation() {
        let rows = FakeRows::threads(&[(7, 3)]);
        let selection = Selection::These(Vec::new());

        let command = command_for(CommandId::Archive, &aim(&selection, Some(7), &rows));

        assert_eq!(
            command.target(),
            Some(&MessageTarget::Thread(ThreadId::new(3))),
        );
    }

    #[test]
    fn a_verb_on_a_message_row_still_means_the_message() {
        let rows = FakeRows::messages(&[7]);
        let selection = Selection::These(Vec::new());

        let command = command_for(CommandId::Archive, &aim(&selection, Some(7), &rows));

        assert_eq!(command.target(), Some(&MessageTarget::Selection));
    }

    #[test]
    fn a_marked_set_of_thread_rows_becomes_a_set_of_conversations() {
        let rows = FakeRows::threads(&[(7, 3), (8, 4), (9, 3)]);
        let selection = Selection::These(vec![message(9), message(7), message(8)]);

        let command = command_for(CommandId::Archive, &aim(&selection, Some(7), &rows));

        assert_eq!(
            command.target(),
            Some(&MessageTarget::Threads(vec![
                ThreadId::new(3),
                ThreadId::new(4)
            ])),
            "sorted and deduped: two rows of one conversation are one target",
        );
    }

    #[test]
    fn a_marked_row_that_is_no_longer_resident_stops_the_whole_conversion() {
        let rows = FakeRows::threads(&[(7, 3)]);
        let selection = Selection::These(vec![message(7), message(8)]);

        let command = command_for(CommandId::Archive, &aim(&selection, Some(7), &rows));

        assert_eq!(
            command.target(),
            Some(&MessageTarget::Selection),
            "guessing that a paged-out row is a conversation would act on \
             mail the user may never have marked",
        );
    }

    #[test]
    fn a_mixed_marked_set_is_left_as_the_messages_it_names() {
        let mut rows = FakeRows::threads(&[(7, 3)]);
        rows.0.insert(8, RowKind::Message);
        let selection = Selection::These(vec![message(7), message(8)]);

        let command = command_for(CommandId::Archive, &aim(&selection, Some(7), &rows));

        assert_eq!(command.target(), Some(&MessageTarget::Selection));
    }

    #[test]
    fn select_all_stays_a_predicate_rather_than_becoming_conversations() {
        let rows = FakeRows::threads(&[(7, 3)]);
        let selection = Selection::Everything {
            except: vec![message(7)],
        };

        let command = command_for(CommandId::Archive, &aim(&selection, Some(7), &rows));

        assert_eq!(
            command.target(),
            Some(&MessageTarget::Selection),
            "resolving the predicate is the mailbox-sized read it exists to \
             avoid",
        );
    }

    #[test]
    fn a_command_that_already_names_its_rows_is_left_alone() {
        let rows = FakeRows::threads(&[(7, 3)]);
        let selection = Selection::These(Vec::new());
        let named = Command::Archive {
            target: MessageTarget::Messages(vec![message(7)]),
        };

        let command = refine(named.clone(), &aim(&selection, Some(7), &rows));

        assert_eq!(command, named);
    }

    #[test]
    fn a_command_with_no_target_at_all_is_left_alone() {
        let rows = FakeRows::threads(&[(7, 3)]);
        let selection = Selection::These(Vec::new());

        let command = command_for(CommandId::NextMessage, &aim(&selection, Some(7), &rows));

        assert_eq!(command, Command::NextMessage);
    }

    #[test]
    fn only_the_commands_the_bus_answers_are_sent_to_it() {
        let wired = vec![CommandId::Archive, CommandId::Undo];
        assert!(is_wired(
            &wired,
            &Command::Archive {
                target: MessageTarget::Selection
            }
        ));
        assert!(is_wired(&wired, &Command::Undo));
        assert!(!is_wired(&wired, &Command::Back));
        assert!(!is_wired(&wired, &Command::Reply { message: None }));
    }

    #[test]
    fn a_bus_with_nothing_wired_is_told_nothing() {
        assert!(!is_wired(
            &[],
            &Command::Archive {
                target: MessageTarget::Selection
            }
        ));
    }

    use crate::state::Resolved;
    use postio_model::MailboxId;

    fn mailbox() -> MailboxId {
        MailboxId::new(4)
    }

    fn mirrored(
        selection: &Selection,
        cursor: Option<i64>,
        scope: Option<ViewScope>,
    ) -> SharedState {
        let state = SharedState::default();
        let (quiet, _reader) = crate::bridge::event_channel();
        let rows = FakeRows::default();
        mirror(
            &state,
            &quiet,
            &Aim {
                scope,
                selection,
                cursor: cursor.map(message),
                rows: &rows,
            },
        );
        state
    }

    /// Clicking a message clears the selection; `a` must still act on the cursor.
    #[test]
    fn an_empty_selection_aims_a_verb_at_the_cursor() {
        let state = mirrored(
            &Selection::These(Vec::new()),
            Some(9),
            Some(ViewScope::Mailbox(mailbox())),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(Resolved::Messages(vec![message(9)])),
        );
    }

    #[test]
    fn a_deliberate_selection_survives_the_mirror() {
        let state = mirrored(
            &Selection::These(vec![message(1), message(2)]),
            Some(9),
            Some(ViewScope::Mailbox(mailbox())),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(Resolved::Messages(vec![message(1), message(2)])),
            "what the user marked, not where they happen to be looking",
        );
    }

    #[test]
    fn select_all_arrives_as_a_predicate_and_keeps_its_exceptions() {
        let state = mirrored(
            &Selection::Everything {
                except: vec![message(7)],
            },
            Some(7),
            Some(ViewScope::Mailbox(mailbox())),
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            Some(Resolved::Everything {
                scope: ViewScope::Mailbox(mailbox()),
                except: vec![message(7)],
            }),
            "resolving it here would be the mailbox-sized read it exists to avoid",
        );
    }

    #[test]
    fn changing_mailbox_does_not_carry_the_old_selection_over() {
        let state = SharedState::default();
        let (quiet, _reader) = crate::bridge::event_channel();
        let rows = FakeRows::default();

        let held = Selection::These(vec![message(1)]);
        mirror(
            &state,
            &quiet,
            &Aim {
                scope: Some(ViewScope::Mailbox(mailbox())),
                selection: &held,
                cursor: Some(message(1)),
                rows: &rows,
            },
        );

        let empty = Selection::These(Vec::new());
        mirror(
            &state,
            &quiet,
            &Aim {
                scope: Some(ViewScope::Mailbox(MailboxId::new(5))),
                selection: &empty,
                cursor: None,
                rows: &rows,
            },
        );

        assert_eq!(
            state.read(|app| app.resolve(&MessageTarget::Selection)),
            None,
        );
    }
    #[test]
    fn the_conformance_table_is_what_command_for_does() {
        let cases = conformance_cases();
        assert!(!cases.is_empty());
        for case in cases {
            let rows = FakeRows(
                case.rows
                    .iter()
                    .copied()
                    .map(|(message, kind)| (message.get(), kind))
                    .collect(),
            );
            let aim = Aim {
                scope: None,
                selection: &case.selection,
                cursor: case.cursor,
                rows: &rows,
            };
            assert_eq!(
                command_for(case.id, &aim),
                case.expected,
                "{}",
                case.because
            );
        }
    }
}
