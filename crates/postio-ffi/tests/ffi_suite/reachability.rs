use postio_ffi::{Session, SessionOptions};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn the_flag_lands_where_the_reader_can_see_it() {
    // `Absent::Offline` versus `Partial` is why this setter exists (#591): a body that is not
    // coming reads differently from one on its way.
    let session = session();
    assert!(
        !session.is_offline(),
        "a fresh session assumes a connection"
    );

    session.set_offline(true);
    assert!(session.is_offline());

    session.set_offline(false);
    assert!(!session.is_offline());
    session.shutdown();
}

#[test]
fn coming_back_online_asks_the_engine_to_try_again() {
    // Without the nudge, waking a laptop waits out whatever backoff step the engine had reached,
    // which can be minutes.
    let session = session();
    session.set_offline(true);
    let before = session.reconnects_for_test();

    session.set_offline(false);
    assert_eq!(
        session.reconnects_for_test(),
        before + 1,
        "regaining the network prompts a reconnect"
    );
    session.shutdown();
}

#[test]
fn nothing_is_nudged_when_the_state_did_not_change() {
    // `NWPathMonitor` repeats itself; reconnecting on every repeat would hammer the server on a
    // flapping connection.
    let session = session();
    session.set_offline(false);
    let before = session.reconnects_for_test();

    session.set_offline(false);
    session.set_offline(false);
    assert_eq!(
        session.reconnects_for_test(),
        before,
        "the same answer twice is not an event"
    );
    session.shutdown();
}

#[test]
fn going_offline_is_never_a_nudge() {
    let session = session();
    let before = session.reconnects_for_test();
    session.set_offline(true);
    assert_eq!(
        session.reconnects_for_test(),
        before,
        "losing the network is not a moment to try connecting"
    );
    session.shutdown();
}
