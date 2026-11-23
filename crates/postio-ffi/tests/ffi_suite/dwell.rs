use chrono::Utc;
use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;
use postio_core::state::SharedState;
use postio_ffi::{Session, SessionOptions};
use postio_model::{Flag, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn is_read(database: &postio_storage::Store, message: i64) -> bool {
    let connection = database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .get(postio_model::ids::MessageId::new(message))
        .await
        .expect("a read")
        .is_some_and(|message| message.flags.contains(&Flag::Seen))
}

async fn one_unread() -> (std::sync::Arc<Session>, postio_storage::Store, i64) {
    let database = test_support::memory().await;
    let message = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.flags.remove(&Flag::Seen);
        repository.create(&mut message).await.expect("a message");
        message.id.get()
    };
    // Real action handlers: an in-memory session's default bridge drops commands, proving only that
    // nothing panicked.
    let state = SharedState::default();
    let bus = postio_session::actions::wire(
        Dispatcher::builder(),
        postio_session::actions::Actions::new(database.clone(), state),
    )
    .build();
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    // Leaked: the bridge must outlive the session.
    let bridge = Box::leak(Box::new(bridge));
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone())
            .on_bridge(bridge.handle(), bridge.commands()),
    )
    .expect("a session over the store");
    (session, database, message)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dwell_marks_the_message_read() {
    let (session, database, message) = one_unread().await;
    assert!(
        !is_read(&database, message).await,
        "the fixture starts unread"
    );

    session.mark_read_on_dwell(message);
    assert!(
        settle_until(async || is_read(&database, message).await).await,
        "the cursor rested on a message and it was never marked read"
    );
    session.shutdown();
}

#[test]
fn the_arming_decision_crosses_whole() {
    // The delay and the rule in one answer: given a bare constant, a frontend wrote the rule again
    // beside it (#1167).
    assert_eq!(
        postio_ffi::dwell_on_cursor(Some(7)),
        postio_ffi::DwellArmFfi::Start {
            message: 7,
            milliseconds: postio_ui::dwell::DWELL_TO_READ.as_millis() as u64,
        }
    );
    assert_eq!(
        postio_ffi::dwell_on_cursor(None),
        postio_ffi::DwellArmFfi::Cancel
    );
}

async fn settle_until<F, Fut>(done: F) -> bool
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    // Scaled so `POSTIO_TEST_PATIENCE` reaches it; a hand-rolled deadline is a flake nobody can
    // reproduce alone (#842).
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        if done().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    done().await
}
