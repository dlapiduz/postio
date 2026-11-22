//! Events: everything the UI reacts to.
//!
//! An event means the local truth already changed, not that the server agreed. Events name entities
//! by id; the list is paged and never materialized whole.

use std::time::Duration;

pub use postio_model::fetch::Sections;
use postio_model::{AccountId, DraftId, MailboxId, MessageId, ThreadId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Offline,
    Connecting,
    Online,
    Failing { reason: FailureReason },
}

/// Categorised by what the user can do about it, never by error text (ADR 0005 Q10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// Never retried on a timer: retrying a rejected credential locks the account.
    Auth,
    NoPassword,
    /// Reserved: ordinary transient trouble stays `Connecting`.
    Network,
    /// Reserved like `Network`: a per-operation server failure is the operation queue's to retry.
    Server,
    Config,
}

/// Every data variant names its account, so a frontend with an aggregated view can tell whose data
/// moved without a store trip (ADR 0005 Q11). `SearchResults` (ranked across a scope) and
/// `ContactsChanged` (people are shared across accounts) are the exceptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    MailboxesChanged {
        account: AccountId,
    },
    MessageListChanged {
        account: AccountId,
        mailbox: MailboxId,
    },
    MessagesChanged {
        account: AccountId,
        messages: Vec<MessageId>,
    },
    MessagesRemoved {
        account: AccountId,
        mailbox: MailboxId,
        messages: Vec<MessageId>,
    },
    NewMail {
        account: AccountId,
        mailbox: MailboxId,
        messages: Vec<MessageId>,
    },
    /// Emitted once per thread per pass, after the transaction commits; `members_added` drives the
    /// new-reply banner.
    ThreadChanged {
        account: AccountId,
        thread: ThreadId,
        /// Empty when the thread changed some other way.
        members_added: Vec<MessageId>,
    },
    BodyLoaded {
        account: AccountId,
        message: MessageId,
        /// A wait for one part listens for its own, and a forwarding composer stops drawing that
        /// part as downloading (#1690).
        sections: Sections,
    },
    /// Read the open message again so "Downloading" becomes "could not be downloaded" (#1918).
    BodyFailed {
        account: AccountId,
        message: MessageId,
        /// A refused text fetch does not end a wait for one of its parts (#1996).
        sections: Sections,
    },
    /// Not a refusal: nothing waiting on a fetch should hear it as one (#1986).
    BodyReachable {
        account: AccountId,
        message: MessageId,
    },
    /// Names no account: Focus's inbox is every account's at once, and a digest can hold mail from
    /// several.
    SurfacedChanged,

    /// Subscribers refetch what is on screen; when `names_changed`, message rows and an open
    /// conversation too, since a name the user set is what those rows draw.
    ContactsChanged {
        names_changed: bool,
    },

    /// Carries the selection, not ids: "select all" is a predicate that must not be flattened
    /// (`state::Selection`).
    SelectionChanged {
        selection: crate::state::Selection,
    },
    /// Carries the view, not a flag, because the frontend must know which thread to render; compose
    /// takes the pane over.
    ViewChanged {
        view: crate::state::ViewMode,
    },
    ContextChanged {
        context: crate::Context,
    },

    /// `took` feeds the < 100 ms local-search budget.
    SearchResults {
        query: String,
        messages: Vec<MessageId>,
        took: Duration,
    },

    ComposerOpened {
        draft: DraftId,
    },
    DraftSaved {
        draft: DraftId,
    },
    MessageSent {
        draft: DraftId,
    },
    ComposerClosed {
        draft: DraftId,
    },

    ConnectionChanged {
        account: AccountId,
        state: ConnectionState,
    },
    SyncProgress {
        account: AccountId,
        done: u32,
        total: u32,
    },
    /// Per mailbox, since other folders keep syncing and `ConnectionState` stays `Online`. Sent
    /// only
    /// on a change; a cancelled pass sends nothing (#1731).
    MailboxSyncChanged {
        account: AccountId,
        mailbox: MailboxId,
        /// Why its last pass failed, phrased for the user; `None` once a
        /// pass has completed again. Never contains a secret or mail.
        failure: Option<String>,
    },
    /// Distinct from `SyncProgress` (the list): a mailbox with an incomplete list cannot be read,
    /// one
    /// with bodies arriving is usable. Pushed because pulling was the only way (#74).
    BackfillProgress {
        account: AccountId,
        done: u32,
        total: u32,
        /// Free to know: `BODYSTRUCTURE` arrives with the header sync (ADR 0017). `None` while
        /// nothing is measured.
        footprint: Option<MailFootprint>,
    },

    /// With `undoable`, this is the "Archived 12 messages - Undo" toast; the description is
    /// user-facing prose.
    ActionCompleted {
        description: String,
        undoable: bool,
    },
    UndoPerformed {
        description: String,
    },
    /// Not an error: the UI usually answers with a quiet hint, not a dialog.
    CommandRejected {
        /// An `ActionId` so a registered extension command is refused through the same event.
        command: crate::ActionId,
        reason: String,
    },
    Error {
        /// The failure, phrased for the user. Never contains a secret.
        message: String,
        /// Frontends with one status line may ignore it; one showing several accounts files the
        /// message under it.
        #[serde(default)]
        account: Option<AccountId>,
    },

    /// Emitted only for `send_tracked` commands, so GTK never sees one. Always arrives, even on a
    /// panic or missing handler: a waiter on a missing answer hangs.
    InvocationFinished {
        invocation: crate::InvocationId,
        outcome: crate::InvocationOutcome,
    },

    /// Names the sections that changed so consumers do only their own work; a save that changes
    /// nothing emits no event.
    ConfigReloaded {
        changed: crate::config::ConfigChange,
    },
}

/// Wire sizes as the server reported them, not the size on disk (blobs are compressed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailFootprint {
    pub total_bytes: u64,
    /// Around 90% on a real account (ADR 0017), which is why the two backfill axes are governed
    /// separately.
    pub attachment_bytes: u64,
    pub local_bytes: u64,
    /// `false` means every number here is a lower bound, and a surface must say so ("over 1.4 GB").
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandId, Context};

    #[test]
    fn events_survive_a_serde_round_trip() {
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
        // Machine-checks the enum doc: a data variant serialises with its account.
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
            Event::ThreadChanged {
                account,
                thread,
                members_added: vec![message],
            },
            Event::BodyLoaded {
                account,
                message,
                sections: Sections::Text,
            },
            Event::BodyFailed {
                account,
                message,
                sections: Sections::Parts(vec!["2".to_owned()]),
            },
            Event::BodyReachable { account, message },
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
