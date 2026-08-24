//! A programmatic caller driving the *application's own* bus.
//!
//! `postio-core`'s correlation tests prove the mechanism over a synthetic
//! dispatcher. This one proves it over the dispatcher `run` composes: the real
//! `Actions` verbs, over a real SQLite store, on a real `Bridge`. The
//! distinction is the one `postio-bl2` was filed about — a mechanism that
//! works when handed its inputs, and an application that never hands them to
//! it, look identical from a unit test.
//!
//! There is no user-facing surface here to reach: `send_tracked` exists for a
//! caller that has no keyboard — MCP, AI, a future CLI. So "can a person reach
//! it" is answered by driving the bus the way such a caller would, and
//! checking both halves: the mail actually moved, *and* the answer came back
//! attributable.
//!
//! Nothing here touches the network. `Actions` is local-first by construction:
//! it writes SQLite and enqueues an operation for the sync engine, which is
//! never started.

use chrono::Utc;
use postio_core::bridge::Bridge;
use postio_core::invocation::InvocationOutcome;
use postio_core::state::SharedState;
use postio_core::{Command, Dispatcher, Event, MessageTarget};
use postio_model::{Message, MessageId};
use postio_session::actions::{Actions, wire};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// The application's bus over a store holding one message in the inbox.
struct World {
    database: postio_storage::Database,
    message: MessageId,
    dispatcher: Dispatcher,
}

fn world() -> World {
    let database = test_support::memory();
    let message = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        test_support::mailbox(&connection, &account, "Archive");
        let mut message = Message::new(account.id, inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("a message")
    };

    let actions = Actions::new(database.clone(), SharedState::default());
    World {
        dispatcher: wire(Dispatcher::builder(), actions).build(),
        database,
        message,
    }
}

impl World {
    fn mailbox_of(&self, message: MessageId) -> postio_model::MailboxId {
        let connection = self.database.connection().expect("a connection");
        MessageRepository::new(&connection)
            .get(message)
            .expect("a read")
            .expect("the message is still there")
            .mailbox_id
    }
}

#[test]
fn a_programmatic_caller_gets_the_answer_to_its_own_archive() {
    let world = world();
    let before = world.mailbox_of(world.message);

    let (bridge, events) = Bridge::new(world.dispatcher.clone()).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::Archive {
            target: MessageTarget::Messages(vec![world.message]),
        })
        .expect("running");
    bridge.shutdown();

    // The verb really ran: this is a real archive, not a mocked one.
    assert_ne!(
        world.mailbox_of(world.message),
        before,
        "the message never moved, so the correlation below would be correlating nothing"
    );

    let ours: Vec<Event> = std::iter::from_fn(|| events.try_next_tracked())
        .filter(|envelope| envelope.is_from(mine))
        .map(|envelope| envelope.event)
        .collect();

    assert!(
        matches!(
            ours.last(),
            Some(Event::InvocationFinished {
                outcome: InvocationOutcome::Completed,
                ..
            })
        ),
        "the caller has no answer to the command it sent: {ours:?}"
    );
    assert!(
        ours.iter()
            .any(|event| matches!(event, Event::MessagesRemoved { .. })),
        "the events the real handler emitted are not attributed to it: {ours:?}"
    );
}

#[test]
fn a_caller_is_told_when_the_application_refuses() {
    // Nothing is selected and no message is named, so the verb has no target.
    // A programmatic caller must hear that rather than wait for it.
    let world = world();
    let before = world.mailbox_of(world.message);
    let (bridge, events) = Bridge::new(world.dispatcher.clone()).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::Archive {
            target: MessageTarget::Selection,
        })
        .expect("running");
    bridge.shutdown();

    let ending = std::iter::from_fn(|| events.try_next_tracked())
        .filter(|envelope| envelope.is_from(mine))
        .find_map(|envelope| match envelope.event {
            Event::InvocationFinished { outcome, .. } => Some(outcome),
            _ => None,
        });

    assert!(
        matches!(ending, Some(InvocationOutcome::Rejected { .. })),
        "an unrunnable command must still end its invocation, got {ending:?}"
    );
    assert_eq!(
        world.mailbox_of(world.message),
        before,
        "a rejected archive must not have moved anything"
    );
}

#[test]
fn the_frontends_own_sends_are_unaffected() {
    // The GTK window sends fire-and-forget and must see exactly the stream it
    // saw before this feature existed: no origins, no completions.
    let world = world();
    let (bridge, events) = Bridge::new(world.dispatcher.clone()).expect("the runtime starts");
    bridge
        .commands()
        .send(Command::Archive {
            target: MessageTarget::Messages(vec![world.message]),
        })
        .expect("running");
    bridge.shutdown();

    let all: Vec<_> = std::iter::from_fn(|| events.try_next_tracked()).collect();
    assert!(!all.is_empty(), "the archive emitted nothing at all");
    assert!(
        all.iter().all(|envelope| envelope.origin.is_none()
            && !matches!(envelope.event, Event::InvocationFinished { .. })),
        "the untracked path grew something: {all:?}"
    );
}
