//! Turning a command id into the invocation it means, here, now.
//!
//! # Why this is in `postio-core` and not in a frontend
//!
//! A [`CommandId`] says *which verb*. It does not say what the verb acts on,
//! and the answer depends on what the user is looking at: a verb on a thread
//! row acts on the conversation ([ADR 0015] Q3), a verb with nothing marked
//! acts on the cursor row, a verb with rows marked acts on all of them. That
//! rule is **semantics**, not presentation, and every frontend has to reach
//! the same answer or `a` means different things on different platforms.
//!
//! It used to live in `postio-app`, coupled to GTK — reaching for widget
//! focus and walking a `GtkListModel` — which meant the macOS boundary had no
//! way to turn `"archive"` into a [`Command`] without writing a **second**
//! mapping. Two mappings would give the two frontends different ideas of what
//! `archive` does the first time either was edited, and the divergence would
//! arrive as a bug report about macOS rather than as a failing test (#589,
//! [ADR 0019]).
//!
//! The third frontend is what settles the crate. [ADR 0010]'s headless MCP
//! server links `postio-session` and no view layer at all, but it does have a
//! selection — and "a verb on a thread row acts on the conversation" has to
//! mean the same thing when MCP archives a thread row as when a person
//! presses `a`. Put the rule in a presentation crate and that frontend either
//! depends on widgets it has no use for or re-implements the rule, which is
//! the drift this module exists to prevent arriving through the door nobody
//! was watching.
//!
//! # Rules are shared; facts cross
//!
//! The seam is [`Aim`]: a borrowed snapshot of what the user is looking at,
//! carrying the selection, the cursor, the view's scope, and one
//! **fact-reporting trait**, [`RowFacts`], which can answer exactly one
//! question — what kind of row is this id. A frontend supplies facts; it
//! never supplies behaviour. A trait with a method like
//! `should_act_on_conversation()` would be this design's failure mode, since
//! it would hand each frontend a decision to make differently.
//!
//! [`RowFacts`] is a trait rather than a snapshot of rows for a reason the
//! whole product rests on: **a mailbox is never materialised**
//! (`docs/PRODUCT.md` §18). A seam that could only be implemented by
//! collecting rows would put that invariant back into each frontend's hands.
//! The old GTK code walked its entire list model building a `BTreeMap` to
//! answer questions about a handful of marked ids; a lookup on the
//! frontend's own index copies nothing.
//!
//! [ADR 0010]: https://github.com/dlapiduz/postio/blob/main/docs/decisions/0010-mcp-surface.md
//! [ADR 0015]: https://github.com/dlapiduz/postio/blob/main/docs/decisions/0015-threaded-list.md
//! [ADR 0019]: https://github.com/dlapiduz/postio/blob/main/docs/decisions/0019-macos-frontend.md

use postio_model::{MessageId, ThreadId};

use crate::bridge::EventSink;
use crate::command::{Command, CommandId, MessageTarget};
use crate::state::{Selection, SharedState, ViewScope};

/// What kind of row an id names, as the frontend's list has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// No resident row with that id — paged out, or gone.
    ///
    /// Load-bearing rather than an error case. A marked row that is no longer
    /// resident cannot be checked, and guessing it is a conversation would
    /// act on mail the user may never have marked (#468).
    Missing,
    /// A message row: a query view, or a list that is not threaded.
    Message,
    /// A conversation row, standing for this thread.
    Thread(ThreadId),
}

/// The only question the shared rules ask a frontend's list.
///
/// One method, answering one fact, so there is nothing in it for a frontend
/// to interpret differently. See the [module docs](self).
pub trait RowFacts {
    /// What kind of row `message` names, if the list still holds one.
    fn row_kind(&self, message: MessageId) -> RowKind;
}

/// What the user is looking at, at the instant a command is issued.
///
/// Borrowed rather than owned: it is built per keystroke from state the
/// frontend already has, and copying a selection that may be a predicate over
/// a whole mailbox to answer one question would be the wrong shape twice.
///
/// # Why there is no `context` here
///
/// [`Context`](crate::context::Context) decides *which id* a chord resolves
/// to, which has already happened by the time anything here runs. It plays no
/// part in deciding what an id acts on: [`refine`]'s rule is narrowed by the
/// row kinds themselves, so a thread drill-in — whose rows are messages, not
/// conversations — is excluded by the facts rather than by the context. A
/// field nothing reads is a field a frontend can set wrongly with nothing to
/// notice, so it is not here until a rule needs it.
pub struct Aim<'a> {
    /// The view a whole-view selection would be relative to.
    ///
    /// `None` when a whole-view gesture is meaningless here — a thread
    /// drill-in, where `Ctrl+A` is not a gesture, or Snoozed, which has no
    /// predicate of its own yet (#493). Each frontend derives this from its
    /// own scope type today; #670 collapses that into one function here.
    pub scope: Option<ViewScope>,
    /// What the user has marked.
    pub selection: &'a Selection,
    /// Where the keyboard is.
    ///
    /// Load-bearing: `AppState::resolve` falls back to it when the selection
    /// is empty, which is the difference between "click a message, press `a`"
    /// archiving that message and archiving nothing at all.
    pub cursor: Option<MessageId>,
    /// The frontend's list, as a source of facts about rows.
    pub rows: &'a dyn RowFacts,
}

/// The invocation `id` means, given what the user is looking at.
///
/// [`Command::default_for`] says what the verb is; [`refine`] says what it
/// acts on. Both frontends call this and neither adds to it.
pub fn command_for(id: CommandId, aim: &Aim<'_>) -> Command {
    refine(Command::default_for(id), aim)
}

/// A verb on a thread row acts on the conversation ([ADR 0015] Q3).
///
/// A folder shows one row per thread, and "the row" is what a key means: `a`
/// archives the conversation, not the one message the row happens to be drawn
/// from. Acting on "the row" and acting on "one message of six" cannot both
/// be what `a` means, and the row is what the user is looking at.
///
/// The conditions are all narrowing:
///
/// * The verb must still be **aimed at the selection**. A hover action or a
///   drop names its own rows and app state is told to take those at their
///   word; re-aiming one would be second-guessing the user's point.
/// * The rows it aims at must **be** thread rows. In a query view they are
///   not, and the verb goes on meaning what it always meant.
///
/// With rows marked it is the same rule over the marked set: a selection of
/// thread rows is a selection of *conversations*. That is the half #307 left
/// and #468 was — a thread row's id is its newest message, so the verbs took
/// a marked set at its word and archived six representatives out of six
/// conversations, leaving the rest of every one of them in the folder after a
/// gesture that looked like it worked.
///
/// [`MessageTarget::Threads`] carries it from here; `Actions::aim` expands
/// the members store-side, which is where [ADR 0015] Q3 says the expansion
/// belongs. A frontend never enumerates a conversation to act on it — it
/// names the conversations and stops.
///
/// [ADR 0015]: https://github.com/dlapiduz/postio/blob/main/docs/decisions/0015-threaded-list.md
pub fn refine(command: Command, aim: &Aim<'_>) -> Command {
    if !matches!(command.target(), Some(MessageTarget::Selection)) {
        return command;
    }
    match aim.selection {
        // Nothing marked: the gesture is about the cursor row.
        Selection::These(marked) if marked.is_empty() => {
            let Some(cursor) = aim.cursor else {
                return command;
            };
            match aim.rows.row_kind(cursor) {
                RowKind::Thread(thread) => command.with_target(MessageTarget::Thread(thread)),
                RowKind::Message | RowKind::Missing => command,
            }
        }
        // Rows marked: every one of them is a conversation (#468).
        Selection::These(marked) => match threads_of(aim.rows, marked) {
            Some(threads) => command.with_target(MessageTarget::Threads(threads)),
            // A set that is not all thread rows — a query view, or a folder
            // mid-switch whose rows have not been replaced yet. Leaving it as
            // `Selection` is today's meaning, and acting on the messages
            // named is never *wrong*, only narrower than a conversation.
            None => command,
        },
        // `Ctrl+A` is a predicate over the view, not a list of rows, and its
        // exceptions are still message ids. Excepting a conversation rather
        // than its representative is the rest of ADR 0015 Q3 and wants
        // `MessageSet` to grow a members-of-these-threads variant, which is
        // its own change — see #468.
        Selection::Everything { .. } => command,
    }
}

/// The conversations `marked` names, or `None` if any of them is not a thread
/// row.
///
/// All-or-nothing on purpose. A half-converted target would archive some rows
/// as conversations and some as single messages from one keystroke, which is
/// a rule nobody could hold in their head — and in a threaded folder every
/// row is a thread row, so the mixed case is a view that is not threaded at
/// all rather than a set worth splitting.
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

/// Point app state at what the user is looking at.
///
/// Every message verb defaults to [`MessageTarget::Selection`], and
/// `AppState::resolve` is what turns that into rows — but the *selection*
/// lives in the frontend's list: it is what the user built with `x`,
/// Ctrl-click and `Ctrl+A`, and the list is the only thing that knows it.
///
/// So app state is brought into step with the view in the instant before a
/// command is sent, rather than being kept in step by a signal. Two reasons.
/// A pull cannot drift: there is no ordering in which the bus resolves
/// against a selection one gesture out of date. And a push would have to fire
/// on every `j`, which is the interaction that happens most and the one with
/// the tightest budget.
///
/// The events this produces have nowhere to go — the view is where they came
/// from, and telling it back would be a round trip to nowhere — so `quiet` is
/// a sink whose reader was dropped on purpose.
pub fn mirror(state: &SharedState, quiet: &EventSink, aim: &Aim<'_>) {
    state.update(quiet, |app| {
        let mut events = Vec::new();
        if let Some(scope) = aim.scope {
            // Opening a different view drops the selection with it, on both
            // sides — the list does the same. So this goes first, or it
            // would throw away the selection just mirrored.
            //
            // The *scope*, not the mailbox it may or may not name: a smart
            // folder has no mailbox, and telling app state only about
            // mailboxes is what left `Ctrl+A` in Flagged with nothing to be
            // relative to (#52).
            events.extend(app.open_view(scope));
        }
        match aim.selection {
            Selection::These(messages) => events.extend(app.select(messages.clone(), aim.cursor)),
            // "Everything" stays a predicate the whole way: the exceptions
            // are re-applied one by one rather than the predicate being
            // resolved into the ids it stands for, because resolving it is
            // exactly the mailbox-sized read it exists to avoid.
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

/// Whether this invocation is the command bus's business.
///
/// The bus is one consumer among several — the composer answers reply and
/// compose, the config module answers `edit_config`, and the window answers
/// `Esc` itself when there is something to close — and it sees every gesture,
/// because the frontend's action seam carries whole invocations. Sending it
/// commands it does not handle would answer a stray `Esc` with "`back` is not
/// wired up in this build".
///
/// `wired` comes from [`Dispatcher::wired`], so this cannot drift from what
/// the bus actually answers.
///
/// [`Dispatcher::wired`]: crate::dispatch::Dispatcher::wired
pub fn is_wired(wired: &[CommandId], command: &Command) -> bool {
    wired.contains(&command.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A list that answers from a map, standing in for a frontend's own
    /// index. No display, no toolkit, and it runs on every host — which is
    /// the point of the rule living here.
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

    /// #468's rule, and the reason `RowKind::Missing` exists at all.
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

    /// A hover action or a drop names its own rows; re-aiming one would be
    /// second-guessing where the user pointed.
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
        let wired = vec![CommandId::Archive];
        assert!(is_wired(
            &wired,
            &Command::Archive {
                target: MessageTarget::Selection
            }
        ));
        assert!(!is_wired(&wired, &Command::Back));
    }
}
