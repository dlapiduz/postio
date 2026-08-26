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
    /// Establishing a connection, or waiting out a backoff that will retry
    /// on its own.
    Connecting,
    /// Connected, with an idle or streaming session.
    Online,
    /// Stopped on something retrying will not fix, and waiting for a person.
    Failing {
        /// What kind of person-shaped problem it is.
        reason: FailureReason,
    },
}

/// Why an account is [`Failing`](ConnectionState::Failing), categorised by
/// what the *user* can do about it — never by error text (ADR 0005 Q10).
///
/// This is what lets a frontend offer "reauthorise this account" for the
/// most common failure instead of guessing from prose, and what routes each
/// kind to the right retry behaviour. The prose still travels beside the
/// state as [`Event::Error`], for the status line to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// The server refused the credential. Re-enter the password or re-run
    /// the sign-in flow. **Never retried on a timer** — retrying a rejected
    /// credential is how an account gets locked; `postio-sync`'s blocked
    /// link holds until the user acts.
    Auth,
    /// The network path to the server is broken in a way backoff has given
    /// up on. Recovers on its own when the path does; nothing for the user
    /// to fix in Postio. (Ordinary transient trouble stays
    /// [`Connecting`](ConnectionState::Connecting); this reason is reserved
    /// for when the supervisor learns to report a backoff that has stopped
    /// making progress.)
    Network,
    /// The server accepted the connection and is refusing the work — out of
    /// quota, a 5xx, a command rejected. Retried, slower. (Reserved like
    /// [`Network`](Self::Network): today a per-operation server failure is
    /// the operation queue's to retry and does not fail the link.)
    Server,
    /// The account's settings are wrong — a certificate that does not
    /// verify, a server with no capabilities, a host that is not an IMAP
    /// server. Retrying cannot fix a setting; the user edits it.
    Config,
}

/// Something the UI needs to react to.
///
/// # Every data variant names its account
///
/// A variant that names a message, mailbox or thread also names the
/// [`AccountId`] it belongs to. With several accounts (ADR 0005 Q11), a
/// frontend keeping an aggregated view has to decide *whose* data moved
/// before it can decide whether a repaint concerns the view on screen — and
/// resolving an id to an account means touching the store, which is the trip
/// the event exists to save. Emitters always know the account already; a new
/// data variant carries it from day one.
///
/// [`SearchResults`](Self::SearchResults) is the deliberate exception:
/// results are ranked across whatever scope the query ran in, so a single
/// account would be wrong in unified scope, and relevance to the view is
/// decided by the query itself. It gains the scope, not an account, with the
/// search-scope work (#186).
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
        /// The account the mailbox belongs to.
        account: AccountId,
        /// The affected mailbox.
        mailbox: MailboxId,
    },
    /// These messages changed in place — flags, labels, read state.
    MessagesChanged {
        /// The account the messages belong to.
        account: AccountId,
        /// The affected messages.
        messages: Vec<MessageId>,
    },
    /// These messages left a mailbox: archived, deleted or moved away.
    MessagesRemoved {
        /// The account the mailbox belongs to.
        account: AccountId,
        /// The mailbox they left.
        mailbox: MailboxId,
        /// The messages that left it.
        messages: Vec<MessageId>,
    },
    /// New mail arrived — the trigger for a desktop notification.
    NewMail {
        /// The account it arrived at.
        account: AccountId,
        /// The mailbox it landed in.
        mailbox: MailboxId,
        /// The newly delivered messages.
        messages: Vec<MessageId>,
    },
    /// A thread gained, lost or re-linked messages after a threading pass.
    ThreadChanged {
        /// The account the thread belongs to.
        account: AccountId,
        /// The affected thread.
        thread: ThreadId,
    },
    /// A message body finished loading from the blob store or the server.
    BodyLoaded {
        /// The account the message belongs to.
        account: AccountId,
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
    /// Progress filling in message *bodies*, for the status line only.
    ///
    /// Distinct from [`SyncProgress`], which is the message *list*. The two
    /// are different phases with different consequences: a mailbox whose list
    /// is incomplete cannot be read at all, and one whose bodies are still
    /// arriving is perfectly usable — so the status line says different
    /// things about them.
    ///
    /// Issue #74: the backfill kept this as a fact nobody could ask for
    /// unprompted, so the longest phase of a first sync was reported as
    /// `idle`. `Engine::backfill_progress` still exists for a caller that
    /// wants to pull; the bug was that pulling was the only way.
    ///
    /// [`SyncProgress`]: Self::SyncProgress
    BackfillProgress {
        /// The account whose bodies these are.
        account: AccountId,
        /// Messages the queue has finished with, one way or another.
        done: u32,
        /// Messages that have entered the queue at all.
        total: u32,
        /// What this account's mail weighs, as the server reported it, and how
        /// much is already here.
        ///
        /// Free to know: `BODYSTRUCTURE` arrives with the header sync, so this
        /// is available before a byte of body is fetched (ADR 0017). It is
        /// what lets a surface say *"890 MB of 1.4 GB"* rather than only
        /// counting messages, and what makes an attachment-policy setting
        /// mean something — "always download attachments" is an abstraction,
        /// "always download attachments: 11.0 GB" is a decision.
        ///
        /// `None` while nothing has been measured yet.
        footprint: Option<MailFootprint>,
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

/// What an account's mail costs, for a surface that has to say so.
///
/// Wire sizes as the server reported them — the answer to "what will this
/// download cost", which is the question a person asks. Not the size on disk:
/// blobs are compressed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailFootprint {
    /// Every byte of every message the account knows about.
    pub total_bytes: u64,
    /// Of that, what attachment payloads account for.
    ///
    /// Around 90% on a real account (ADR 0017), which is why the two backfill
    /// axes are governed separately and why this number is worth showing next
    /// to the setting that decides whether they are fetched.
    pub attachment_bytes: u64,
    /// What is already downloaded.
    pub local_bytes: u64,
    /// Whether every selectable mailbox has finished a header pass.
    ///
    /// **`false` means every number here is a lower bound**, and a surface
    /// must say so — "over 1.4 GB", not "1.4 GB". A total that silently climbs
    /// every few seconds reads as a bug, and one that is simply wrong is worse
    /// than one that admits it is still counting.
    pub complete: bool,
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
    fn a_failing_state_says_what_kind_of_help_it_needs() {
        // ADR 0005 Q10: the reason is typed and on the event, so a frontend
        // can offer "reauthorise" without parsing prose.
        let event = Event::ConnectionChanged {
            account: AccountId::new(1),
            state: ConnectionState::Failing {
                reason: FailureReason::Auth,
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"auth\""), "{json}");
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }

    #[test]
    fn every_data_variant_names_its_account() {
        // The rule in the enum's doc comment, machine-checked: a variant that
        // names data serialises with the account it belongs to. A new data
        // variant that forgets the field fails here, not in a review.
        let account = AccountId::new(7);
        let mailbox = MailboxId::new(1);
        let message = MessageId::new(2);
        let thread = ThreadId::new(3);
        let events = [
            Event::MailboxesChanged { account },
            Event::MessageListChanged { account, mailbox },
            Event::MessagesChanged {
                account,
                messages: vec![message],
            },
            Event::MessagesRemoved {
                account,
                mailbox,
                messages: vec![message],
            },
            Event::NewMail {
                account,
                mailbox,
                messages: vec![message],
            },
            Event::ThreadChanged { account, thread },
            Event::BodyLoaded { account, message },
        ];
        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"account\":7"), "{json}");
            assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
        }
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
