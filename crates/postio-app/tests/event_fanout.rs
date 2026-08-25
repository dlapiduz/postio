//! A second consumer beside the window, over the bus `run` actually composes.
//!
//! `postio-core`'s `event_hub.rs` proves the hub over a synthetic dispatcher.
//! This one proves the arrangement ADR 0010 needs — "MCP is a second frontend
//! over `postio-core`'s bridge", sitting beside a running window — over the
//! real `Actions` verbs, a real SQLite store and a real `Bridge`. That is the
//! `postio-bl2` distinction: a mechanism that works when handed its inputs,
//! and an application that never hands them to it, look identical from a unit
//! test.
//!
//! The window's own reachability is the other half, and it is structural: the
//! window drains **one** subscription now, where it used to collect a
//! `Vec<Option<EventStream>>` from two producers. If that subscription ever
//! stopped carrying the engine's events, mail would arrive and the list would
//! not move — so this holds both producers on one stream explicitly.
//!
//! Nothing here touches the network: `Actions` is local-first, and the sync
//! engine is never started.

use chrono::Utc;
use postio_core::bridge::{Bridge, EventHub, EventStream};
use postio_core::invocation::{EventEnvelope, InvocationOutcome};
use postio_core::state::SharedState;
use postio_core::{Command, Event, MessageTarget};
use postio_model::{Message, MessageId};
use postio_session::actions::{Actions, wire};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

struct World {
    database: postio_storage::Database,
    message: MessageId,
    dispatcher: postio_core::Dispatcher,
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
        database,
        message,
        dispatcher: wire(postio_core::Dispatcher::builder(), actions).build(),
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

fn drain(events: &EventStream) -> Vec<EventEnvelope> {
    std::iter::from_fn(|| events.try_next_tracked()).collect()
}

/// The engine's shape of event: a producer that is not a command handler and
/// so is never handed a sink by the bridge.
fn engine_noise() -> Event {
    Event::ConnectionChanged {
        account: postio_model::AccountId::new(1),
        state: postio_core::ConnectionState::Online,
    }
}

#[test]
fn a_second_frontend_sees_everything_the_window_sees() {
    let world = world();
    let before = world.mailbox_of(world.message);

    // Exactly `run`'s arrangement: one hub, the bus built on a sink from it,
    // the engine holding another, and each consumer subscribing for itself.
    let hub = EventHub::new();
    let engine = hub.sink();
    let window = hub.subscribe("window");
    let mcp = hub.subscribe("mcp");
    let bridge = Bridge::builder()
        .build_with_events(world.dispatcher.clone(), hub.sink())
        .expect("the runtime starts");

    let mine = bridge
        .commands()
        .send_tracked(Command::Archive {
            target: MessageTarget::Messages(vec![world.message]),
        })
        .expect("running");
    engine.emit(engine_noise());
    bridge.shutdown();

    // The verb really ran, so what follows is correlating something.
    assert_ne!(world.mailbox_of(world.message), before, "nothing moved");

    for (label, events) in [("window", &window), ("mcp", &mcp)] {
        let all = drain(events);

        // The engine's events reach this subscriber too. Before the hub the
        // engine had a channel of its own and a consumer had to be handed it
        // separately; a second frontend would have gone blind to every socket
        // state change in the application.
        assert!(
            all.iter().any(|it| it.event == engine_noise()),
            "{label} never saw the producer that is not a command handler: {all:?}"
        );

        let ours: Vec<Event> = all
            .iter()
            .filter(|envelope| envelope.is_from(mine))
            .map(|envelope| envelope.event.clone())
            .collect();
        assert!(
            matches!(
                ours.last(),
                Some(Event::InvocationFinished {
                    outcome: InvocationOutcome::Completed,
                    ..
                })
            ),
            "{label} has no answer to the tracked send: {ours:?}"
        );
        assert!(
            ours.iter()
                .any(|event| matches!(event, Event::MessagesRemoved { .. })),
            "{label} did not see the real handler's own events: {ours:?}"
        );
        // The hub filters nothing (ADR 0013 Q3), so the engine's untagged
        // event must not be attributed to anyone's invocation.
        assert!(
            !ours.iter().any(|event| *event == engine_noise()),
            "{label} attributed the engine's event to a command: {ours:?}"
        );
    }
}

#[test]
fn one_subscription_carries_both_of_the_applications_producers() {
    // The fan-in half, and the reason the `Vec<Option<EventStream>>` handoff
    // could go: the window used to collect one stream per producer by hand.
    let world = world();
    let hub = EventHub::new();
    let engine = hub.sink();
    let window = hub.subscribe("window");
    let bridge = Bridge::builder()
        .build_with_events(world.dispatcher.clone(), hub.sink())
        .expect("the runtime starts");

    engine.emit(engine_noise());
    bridge
        .commands()
        .send(Command::Archive {
            target: MessageTarget::Messages(vec![world.message]),
        })
        .expect("running");
    bridge.shutdown();

    let all = drain(&window);
    assert!(
        all.iter().any(|it| it.event == engine_noise()),
        "the engine's half is missing from the window's one stream: {all:?}"
    );
    assert!(
        all.iter()
            .any(|it| matches!(it.event, Event::MessagesRemoved { .. })),
        "the bus's half is missing from the window's one stream: {all:?}"
    );
}
