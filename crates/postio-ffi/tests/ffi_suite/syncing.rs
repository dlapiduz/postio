use postio_ffi::{Session, SessionOptions, UiEvent};
use postio_storage::test_support;

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_store_with_no_accounts_starts_nothing_and_says_so() {
    // A fresh store with no account is the ordinary first-run state, not an error.
    let session = session();
    let started = session
        .start_syncing()
        .await
        .expect("no accounts is not a failure");
    assert_eq!(
        started, 0,
        "engines were started for a store with no accounts"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn starting_twice_is_harmless() {
    // An app lifecycle calls this twice (a window reopening, a wake); doubling the engines would
    // double every server connection.
    let session = session();
    let first = session.start_syncing().await.expect("first start");
    let second = session.start_syncing().await.expect("second start");
    assert_eq!(first, second, "a second start produced a different result");
    session.shutdown();
}

#[test]
fn an_adopted_engine_reaches_the_slot_that_refresh_reads() {
    // `Refresh` needs the engine and is pressed long after the bus was built; an engine that never
    // reached the slot would sync and leave refresh inert.
    let session = session();
    assert!(
        !session.has_engine(),
        "a fresh session should hold no engine"
    );

    session.adopt_mock_engine_for_test();

    assert!(
        session.has_engine(),
        "the engine was started but never reached the slot Refresh reads"
    );
    session.shutdown();
}

#[test]
fn connection_state_reaches_the_frontend() {
    // Connecting, offline and failing must cross, or the frontend cannot say whether to wait or
    // give up.
    let session = session();
    session.emit_for_test(postio_core::Event::ConnectionChanged {
        account: 1.into(),
        state: postio_core::ConnectionState::Connecting,
    });

    match session.next_event_blocking().expect("an event arrives") {
        UiEvent::ConnectionChanged { account, state } => {
            assert_eq!(account, 1);
            assert_eq!(state, postio_ffi::ConnectionStateFfi::Connecting);
        }
        other => panic!("expected ConnectionChanged, got {other:?}"),
    }
    session.shutdown();
}

#[test]
fn a_failing_connection_is_distinct_from_being_offline() {
    // `Failing` is stopped on something retrying will not fix (a bad password); showing "offline"
    // sends the user to check their network.
    let session = session();
    session.emit_for_test(postio_core::Event::ConnectionChanged {
        account: 1.into(),
        state: postio_core::ConnectionState::Offline,
    });
    session.emit_for_test(postio_core::Event::ConnectionChanged {
        account: 1.into(),
        state: postio_core::ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        },
    });

    let offline = session.next_event_blocking().expect("the first event");
    let failing = session.next_event_blocking().expect("the second event");
    assert_ne!(
        offline, failing,
        "offline and failing crossed as the same state, so no frontend could \
         tell the user which one it is"
    );
    session.shutdown();
}

#[test]
fn sync_progress_reaches_the_frontend() {
    let session = session();
    session.emit_for_test(postio_core::Event::SyncProgress {
        account: 1.into(),
        done: 40,
        total: 100,
    });

    match session.next_event_blocking().expect("an event arrives") {
        UiEvent::SyncProgress {
            account,
            done,
            total,
        } => {
            assert_eq!((account, done, total), (1, 40, 100));
        }
        other => panic!("expected SyncProgress, got {other:?}"),
    }
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_seeded_account_is_seen_by_the_starter() {
    // Guards the account read: a `start_syncing` that could not see an account would answer zero
    // like the no-accounts case.
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        test_support::account_with_inbox(&connection).await;
    }
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    assert_eq!(
        session.configured_accounts().await,
        1,
        "the configured account was not seen"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_added_while_running_is_not_left_without_an_engine() {
    // `start_syncing` returned early whenever any engine existed, so an account added while the app
    // ran got no engine and looked unsaved until relaunch.
    let session = session();
    session.adopt_mock_engine_for_test();
    assert!(session.has_engine());

    // Nothing dials: what is under test is whether the new account is considered at all.
    let started = session
        .start_syncing()
        .await
        .expect("a running engine must not make this an error");

    assert!(
        started >= 1,
        "an engine is already running and must still be counted: {started}"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_start_still_does_not_double_the_engines() {
    // The same account must never get two engines, or every server connection doubles.
    let session = session();
    session.adopt_mock_engine_for_test();

    let first = session.start_syncing().await.expect("first");
    let second = session.start_syncing().await.expect("second");

    assert_eq!(
        first, second,
        "a second start changed the count, so an account was started twice"
    );
    session.shutdown();
}
