//! Starting the engine, and what the frontend hears while it runs.
//!
//! The macOS application opened a store and it stayed empty forever, because
//! nothing on this boundary ever started a sync. The store being empty was
//! never a rendering problem; nothing had fetched anything.
//!
//! Nothing here dials: the engine is driven against `MockBackend`, which is
//! the seam CLAUDE.md names for exactly this.

use postio_ffi::{Session, SessionOptions, UiEvent};
use postio_storage::test_support;

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn a_store_with_no_accounts_starts_nothing_and_says_so() {
    // Not an error. A fresh store with no account configured is the ordinary
    // first-run state, and treating it as a failure would put an error on
    // screen for someone who has simply not finished setting up.
    let session = session();
    let started = session
        .start_syncing()
        .expect("no accounts is not a failure");
    assert_eq!(
        started, 0,
        "engines were started for a store with no accounts"
    );
    session.shutdown();
}

#[test]
fn starting_twice_is_harmless() {
    // `postio-app`'s own comment records that a second `start_syncing` used
    // to run a duplicate pass. An application lifecycle will call this twice
    // — a window reopening, a wake from sleep — and doubling the engines
    // would double every connection to the server.
    let session = session();
    let first = session.start_syncing().expect("first start");
    let second = session.start_syncing().expect("second start");
    assert_eq!(first, second, "a second start produced a different result");
    session.shutdown();
}

#[test]
fn an_adopted_engine_reaches_the_slot_that_refresh_reads() {
    // The half of adoption that is this boundary's own. `Refresh` is the one
    // command that needs the engine and it is pressed long after the bus was
    // built, so an engine that ran but never reached the slot would sync
    // happily and leave the refresh command inert.
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
    // A first run is the longest the application ever spends looking broken,
    // and "connecting" against "offline" against "failing" is the whole
    // difference between waiting and giving up. If these do not cross, the
    // frontend has no way to say which one is happening.
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
    // `Failing` means "stopped on something retrying will not fix, waiting
    // for a person" — a bad password, usually. Rendering it as "offline"
    // tells the user to check their network when the answer is to check
    // their credentials, and they will wait indefinitely for a reconnect
    // that is never coming.
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
    // The only thing a first run has to show that something is happening. A
    // backfill of a large mailbox is minutes of nothing otherwise.
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

#[test]
fn a_seeded_account_is_seen_by_the_starter() {
    // Guards the account read itself: a `start_syncing` that could not see a
    // configured account would answer zero for ever and look exactly like the
    // no-accounts case above.
    let database = test_support::memory();
    {
        let connection = database.connection().expect("a connection");
        test_support::account_with_inbox(&connection);
    }
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    assert_eq!(
        session.configured_accounts(),
        1,
        "the configured account was not seen"
    );
    session.shutdown();
}
