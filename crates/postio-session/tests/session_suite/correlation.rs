use chrono::Utc;
use postio_core::bridge::Bridge;
use postio_core::invocation::InvocationOutcome;
use postio_core::state::SharedState;
use postio_core::{Command, Dispatcher, Event, MessageTarget};
use postio_model::{Message, MessageId};
use postio_session::actions::{Actions, wire};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

struct World {
    database: postio_storage::Store,
    message: MessageId,
    dispatcher: Dispatcher,
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
        dispatcher: wire(Dispatcher::builder(), actions).build(),
        database,
        message,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_programmatic_caller_gets_the_answer_to_its_own_archive() {
    let world = world().await;
    let before = world.mailbox_of(world.message).await;

    let (bridge, events) = Bridge::new(world.dispatcher.clone()).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::Archive {
            target: MessageTarget::Messages(vec![world.message]),
        })
        .expect("running");
    bridge.shutdown();

    assert_ne!(
        world.mailbox_of(world.message).await,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_caller_is_told_when_the_application_refuses() {
    let world = world().await;
    let before = world.mailbox_of(world.message).await;
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
        world.mailbox_of(world.message).await,
        before,
        "a rejected archive must not have moved anything"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_frontends_own_sends_are_unaffected() {
    let world = world().await;
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
