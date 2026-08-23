//! The message verbs, as the command bus runs them.
//!
//! Archive, delete, move, flag and mark-unread are the daily vocabulary, and
//! every one of them takes the same shape: resolve what the invocation is
//! aimed at, write SQLite, enqueue the operation the server will eventually
//! see, push an undo entry, emit the events the panes repaint from. Nothing
//! here awaits the network — `postio-sync` drains the queue on its own, later
//! and somewhere else — which is what makes these work on a train.
//!
//! # Why they live in the composition root
//!
//! A handler needs the store, and `postio-core` is not allowed to know what
//! SQLite is. `postio-gtk` is not allowed to either. This crate is the one
//! that knows both halves exist, so this is where the verb meets the
//! database.
//!
//! # `run` takes a command, not an `Invocation`
//!
//! [`Invocation`](postio_core::dispatch::Invocation) can only be built by the
//! bus, so a handler written against it can only be tested through a running
//! tokio runtime. Taking the command and the sink instead makes every verb an
//! ordinary function over a throwaway database, and leaves the registration
//! below as the only thing the bus shape touches.

use std::sync::{Arc, Mutex};

use postio_core::bridge::EventSink;
use postio_core::dispatch::{CommandError, Dispatcher};
use postio_core::state::SharedState;
use postio_core::undo::UndoStack;
use postio_core::{Command, CommandId, Event};
use postio_storage::Database;

/// The commands this module answers.
///
/// The composition root does not need this — [`Dispatcher::handles`] answers
/// the same question — but naming them once keeps the registration and the
/// match in [`Actions::run`] from drifting apart.
const WIRED: &[CommandId] = &[CommandId::Undo];

/// Everything a verb needs: the store to write, the state to resolve against,
/// and the history to push onto.
///
/// Cheap to clone — a `Database` is a connection pool behind a handle, and the
/// other two are shared by construction — because the bus holds one of these
/// per registered command.
#[derive(Clone)]
pub struct Actions {
    database: Database,
    state: SharedState,
    undo: Arc<Mutex<UndoStack>>,
}

impl Actions {
    /// Verbs over `database`, resolving their targets against `state`.
    pub fn new(database: Database, state: SharedState) -> Self {
        Actions {
            database,
            state,
            undo: Arc::new(Mutex::new(UndoStack::new())),
        }
    }

    /// Run one invocation, reporting through `events`.
    ///
    /// `Err` is what the user sees as a quiet hint or a visible failure; the
    /// bus turns it into [`Event::CommandRejected`] or [`Event::Error`].
    pub fn run(&self, command: &Command, events: &EventSink) -> Result<(), CommandError> {
        match command {
            Command::Undo => self.undo(events),
            other => Err(CommandError::rejected(format!(
                "`{}` is not wired up yet",
                other.id()
            ))),
        }
    }

    /// Take back the last undoable unit.
    ///
    /// Nothing to take back is a rejection rather than a failure: pressing `u`
    /// on a fresh session is an ordinary thing to do, and it deserves a
    /// sentence, not a dialog.
    fn undo(&self, events: &EventSink) -> Result<(), CommandError> {
        let entry = self
            .stack()
            .undo()
            .ok_or_else(|| CommandError::rejected("Nothing to undo"))?;
        events.emit(Event::UndoPerformed {
            description: entry.description(),
        });
        Ok(())
    }

    fn stack(&self) -> std::sync::MutexGuard<'_, UndoStack> {
        // A panicking handler must not cost the application its history; the
        // bus has already reported the panic as an error event.
        self.undo
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Silences the unused-field warning until the verbs land. The store is
    /// what this struct exists to carry.
    #[allow(dead_code)]
    fn database(&self) -> &Database {
        &self.database
    }

    /// As above, for the state the targets resolve against.
    #[allow(dead_code)]
    fn state(&self) -> &SharedState {
        &self.state
    }
}

/// The bus, with every verb this module answers registered on it.
pub fn dispatcher(actions: Actions) -> Dispatcher {
    Dispatcher::builder()
        .on_each(WIRED.iter().copied(), move |invocation| {
            let actions = actions.clone();
            // Synchronous on purpose: a local-first verb is one indexed write
            // and one queue row, and the bus awaits each handler so that app
            // state and the undo stack see a total order. Anything that could
            // actually take time belongs on a spawned task reporting through
            // its own events.
            async move { actions.run(&invocation.command, &invocation.events()) }
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::bridge::event_channel;

    fn actions() -> Actions {
        Actions::new(
            postio_storage::test_support::memory(),
            SharedState::default(),
        )
    }

    #[test]
    fn undo_with_nothing_to_take_back_is_a_rejection() {
        let (sink, events) = event_channel();

        let outcome = actions().run(&Command::Undo, &sink);

        assert_eq!(
            outcome,
            Err(CommandError::rejected("Nothing to undo")),
            "a quiet hint, not a failure: `u` on a fresh session is ordinary"
        );
        assert!(events.try_next().is_none(), "and nothing happened");
    }

    #[test]
    fn a_verb_with_no_handler_says_so_rather_than_going_quiet() {
        let (sink, _events) = event_channel();

        let outcome = actions().run(
            &Command::Archive {
                target: postio_core::MessageTarget::Selection,
            },
            &sink,
        );

        assert!(matches!(outcome, Err(CommandError::Rejected(_))));
    }

    #[test]
    fn every_wired_command_has_a_handler_and_answers() {
        // A registry entry with no handler is a palette row that does
        // nothing; a handler for a command `run` does not match is a
        // keystroke that reports "not wired up yet" from inside the thing
        // that is supposed to be wiring it up.
        let bus = dispatcher(actions());

        assert_eq!(bus.wired().collect::<Vec<_>>(), WIRED.to_vec());
    }
}
