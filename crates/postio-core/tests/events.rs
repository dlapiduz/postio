//! `Event` is everything the UI reacts to. The frontend never polls state and
//! never awaits the network, so anything a widget must repaint for has to be
//! expressible here.

use std::time::Duration;

use postio_core::{Command, CommandId, ConnectionState, Context, Event, MessageTarget};
use postio_model::{AccountId, DraftId, MailboxId, MessageId, ThreadId};

#[test]
fn events_cover_the_repaint_surface() {
    let events = vec![
        Event::MailboxesChanged {
            account: AccountId::new(1),
        },
        Event::MessageListChanged {
            account: AccountId::new(1),
            mailbox: MailboxId::new(2),
        },
        Event::MessagesChanged {
            account: AccountId::new(1),
            messages: vec![MessageId::new(3)],
        },
        Event::MessagesRemoved {
            account: AccountId::new(1),
            mailbox: MailboxId::new(2),
            messages: vec![MessageId::new(3)],
        },
        Event::NewMail {
            account: AccountId::new(1),
            mailbox: MailboxId::new(2),
            messages: vec![MessageId::new(4)],
        },
        Event::ThreadChanged {
            account: AccountId::new(1),
            thread: ThreadId::new(5),
        },
        Event::BodyLoaded {
            account: AccountId::new(1),
            message: MessageId::new(3),
        },
        Event::SelectionChanged {
            selection: postio_core::state::Selection::These(vec![MessageId::new(3)]),
        },
        Event::ContextChanged {
            context: Context::Reader,
        },
        Event::SearchResults {
            query: "from:alice".into(),
            messages: vec![MessageId::new(3)],
            took: Duration::from_millis(4),
        },
        Event::ComposerOpened {
            draft: DraftId::new(6),
        },
        Event::DraftSaved {
            draft: DraftId::new(6),
        },
        Event::MessageSent {
            draft: DraftId::new(6),
        },
        Event::ComposerClosed {
            draft: DraftId::new(6),
        },
        Event::ConnectionChanged {
            account: AccountId::new(1),
            state: ConnectionState::Online,
        },
        Event::SyncProgress {
            account: AccountId::new(1),
            done: 2,
            total: 10,
        },
        Event::ActionCompleted {
            description: "Archived 12 messages".into(),
            undoable: true,
        },
        Event::UndoPerformed {
            description: "Archived 12 messages".into(),
        },
        Event::ConfigReloaded {
            changed: postio_core::ConfigChange::default(),
        },
        Event::CommandRejected {
            command: CommandId::Undo.into(),
            reason: "nothing to undo".into(),
        },
        Event::Error {
            message: "connection refused".into(),
        },
    ];

    // Clone + PartialEq + Debug: the GTK layer diffs and logs these.
    for event in &events {
        assert_eq!(event.clone(), *event);
        assert!(!format!("{event:?}").is_empty());
    }
}

#[test]
fn an_undoable_action_announces_itself() {
    // docs/PRODUCT.md §16: "Archived 12 messages — Undo" is driven by this pairing.
    let command = Command::Archive {
        target: MessageTarget::Selection,
    };
    assert!(command.is_destructive());
    let event = Event::ActionCompleted {
        description: "Archived 12 messages".into(),
        undoable: true,
    };
    match event {
        Event::ActionCompleted {
            undoable,
            description,
        } => {
            assert!(undoable);
            assert!(description.contains("Archived"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
