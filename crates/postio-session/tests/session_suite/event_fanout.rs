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
    database: postio_storage::Store,
    message: MessageId,
    dispatcher: postio_core::Dispatcher,
}

async fn world() -> World {
    let database = test_support::memory().await;
    let message = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        test_support::mailbox(&connection, &account, "Archive").await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
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
    async fn mailbox_of(&self, message: MessageId) -> postio_model::MailboxId {
        let connection = self.database.connect().await.expect("a connection");
        MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("a read")
            .expect("the message is still there")
            .mailbox_id
    }
}

fn drain(events: &EventStream) -> Vec<EventEnvelope> {
    std::iter::from_fn(|| events.try_next_tracked()).collect()
}

fn engine_noise() -> Event {
    Event::ConnectionChanged {
        account: postio_model::AccountId::new(1),
        state: postio_core::ConnectionState::Online,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_frontend_sees_everything_the_window_sees() {
    let world = world().await;
    let before = world.mailbox_of(world.message).await;

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

    assert_ne!(
        world.mailbox_of(world.message).await,
        before,
        "nothing moved"
    );

    for (label, events) in [("window", &window), ("mcp", &mcp)] {
        let all = drain(events);

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
        assert!(
            !ours.iter().any(|event| *event == engine_noise()),
            "{label} attributed the engine's event to a command: {ours:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_subscription_carries_both_of_the_applications_producers() {
    let world = world().await;
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
