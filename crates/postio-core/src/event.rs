//! Events: everything the UI reacts to.
//!
//! The frontend does not poll and does not query state on a timer — it repaints
//! when an [`Event`] arrives. Every mutating action is local-first (SQLite
//! write, enqueue the operation, emit the event, repaint), so an event means
//! *the local truth already changed*, not *the server agreed*. Reconciliation
//! with the server arrives later as further events.
//!
//! Events name entities by id rather than carrying loaded rows: the message
//! list is windowed over paged SQLite and must never be materialized whole.

use std::time::Duration;

use postio_model::{AccountId, DraftId, MailboxId, MessageId, ThreadId};
use serde::{Deserialize, Serialize};

/// Where an account stands with its server.
///
/// A summary for the status line, deliberately not the sync engine's internal
/// state machine — `postio-core` should not have to change when that does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Working from the local database only.
    Offline,
    /// Establishing a connection.
    Connecting,
    /// Connected, with an idle or streaming session.
    Online,
    /// The last attempt failed; the engine is backing off.
    Failing,
}

/// Something the UI needs to react to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    // -- Data ------------------------------------------------------------
    /// An account's mailbox tree changed: renamed, created, subscribed.
    MailboxesChanged {
        /// The account whose mailboxes changed.
        account: AccountId,
    },
    /// A mailbox's message list changed enough that the window must reload.
    MessageListChanged {
        /// The affected mailbox.
        mailbox: MailboxId,
    },
    /// These messages changed in place — flags, labels, read state.
    MessagesChanged {
        /// The affected messages.
        messages: Vec<MessageId>,
    },
    /// These messages left a mailbox: archived, deleted or moved away.
    MessagesRemoved {
        /// The mailbox they left.
        mailbox: MailboxId,
        /// The messages that left it.
        messages: Vec<MessageId>,
    },
    /// New mail arrived — the trigger for a desktop notification.
    NewMail {
        /// The mailbox it landed in.
        mailbox: MailboxId,
        /// The newly delivered messages.
        messages: Vec<MessageId>,
    },
    /// A thread gained, lost or re-linked messages after a threading pass.
    ThreadChanged {
        /// The affected thread.
        thread: ThreadId,
    },
    /// A message body finished loading from the blob store or the server.
    BodyLoaded {
        /// The message whose body is now available.
        message: MessageId,
    },

    // -- View ------------------------------------------------------------
    /// The selection changed, by keyboard or pointer.
    ///
    /// Carries the selection rather than a list of ids: "select all" in a
    /// large mailbox is a predicate, and an event that flattened it would
    /// undo the whole reason it is one. See [`crate::state::Selection`].
    SelectionChanged {
        /// What an action would now hit.
        selection: crate::state::Selection,
    },
    /// The reading pane changed what it is showing: the list, a thread, a
    /// message, search results or the composer.
    ///
    /// Carries the view rather than just a flag because the frontend has to
    /// know *which* thread to render, and because compose takes the pane over
    /// rather than opening a window of its own.
    ViewChanged {
        /// What the pane is showing now.
        view: crate::state::ViewMode,
    },
    /// The keyboard context changed, so the key hints and palette must refilter.
    ContextChanged {
        /// The context that now owns the keyboard.
        context: crate::Context,
    },

    // -- Search ----------------------------------------------------------
    /// A search finished. `took` feeds the < 100 ms local-search budget.
    SearchResults {
        /// The query these results answer.
        query: String,
        /// The matching messages, most relevant first.
        messages: Vec<MessageId>,
        /// How long the search took.
        took: Duration,
    },

    // -- Compose ---------------------------------------------------------
    /// The composer took over the reading pane.
    ComposerOpened {
        /// The draft being edited.
        draft: DraftId,
    },
    /// A draft was written to the local database.
    DraftSaved {
        /// The saved draft.
        draft: DraftId,
    },
    /// A message was handed to the send queue.
    MessageSent {
        /// The draft that became the message.
        draft: DraftId,
    },
    /// The composer gave the reading pane back.
    ComposerClosed {
        /// The draft that was open.
        draft: DraftId,
    },

    // -- Sync ------------------------------------------------------------
    /// An account's connection state changed.
    ConnectionChanged {
        /// The account.
        account: AccountId,
        /// Its new state.
        state: ConnectionState,
    },
    /// Progress on a long resynchronization, for the status line only.
    SyncProgress {
        /// The account being synchronized.
        account: AccountId,
        /// Units completed.
        done: u32,
        /// Units expected.
        total: u32,
    },

    // -- Feedback --------------------------------------------------------
    /// An action finished and should be announced.
    ///
    /// With `undoable`, this is the *"Archived 12 messages — Undo"* toast of
    /// docs/PRODUCT.md §16; the description is already user-facing prose.
    ActionCompleted {
        /// What happened, phrased for the user.
        description: String,
        /// Whether the undo stack can take it back.
        undoable: bool,
    },
    /// An undo was applied.
    UndoPerformed {
        /// What was taken back, phrased for the user.
        description: String,
    },
    /// A command could not run — nothing to undo, no selection, offline.
    ///
    /// Not an error: the UI usually answers with a quiet hint, not a dialog.
    CommandRejected {
        /// The command that was refused.
        ///
        /// An [`ActionId`](crate::ActionId) rather than a `CommandId`, so a
        /// registered extension command is refused through the same event and
        /// answered by the same quiet hint. It still serialises as the
        /// command's stable string id.
        command: crate::ActionId,
        /// Why, phrased for the user.
        reason: String,
    },
    /// Something failed and the user should know.
    Error {
        /// The failure, phrased for the user. Never contains a secret.
        message: String,
    },

    /// A tracked invocation ended, whichever way it ended.
    ///
    /// Emitted only for a command sent through
    /// [`CommandSender::send_tracked`](crate::bridge::CommandSender::send_tracked),
    /// so the GTK frontend — which sends fire-and-forget — never sees one and
    /// does not gain an event per keystroke.
    ///
    /// It is the *terminal* event of an invocation and always arrives, even
    /// when the handler panicked or no handler existed: a programmatic caller
    /// awaiting an answer that never comes is a hang, which is a worse failure
    /// than the one it was reporting. The user-facing prose has already gone
    /// past as [`Event::CommandRejected`] or [`Event::Error`]; this repeats it
    /// so one event answers the whole question.
    InvocationFinished {
        /// The send this answers.
        invocation: crate::InvocationId,
        /// How it ended.
        outcome: crate::InvocationOutcome,
    },

    // -- Configuration ---------------------------------------------------
    /// `config.toml` was reloaded and something a subsystem cares about moved.
    ///
    /// Carries which sections changed so consumers can do only their own work:
    /// reapplying everything on every keystroke in `$EDITOR` would be visibly
    /// slow. A save that changes nothing emits no event at all.
    ConfigReloaded {
        /// The sections that moved.
        changed: crate::config::ConfigChange,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandId, Context};

    #[test]
    fn events_survive_a_serde_round_trip() {
        // Events are logged and replayed in tests; they must round-trip.
        let event = Event::CommandRejected {
            command: CommandId::Undo.into(),
            reason: "nothing to undo".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"undo\""), "{json}");
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }

    #[test]
    fn a_completion_names_the_send_it_answers() {
        let event = Event::InvocationFinished {
            invocation: crate::InvocationId::next(),
            outcome: crate::InvocationOutcome::Completed,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("completed"), "{json}");
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }

    #[test]
    fn a_context_change_carries_the_new_context() {
        let event = Event::ContextChanged {
            context: Context::Composer,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("composer"), "{json}");
    }
}
