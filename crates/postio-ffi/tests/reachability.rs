//! Reachability pushed down from the platform (#663).
//!
//! The observation happens in Swift, where `NWPathMonitor` reads it in four
//! lines; Rust is told. Binding it here would mean `unsafe` in a crate that
//! forbids it, for a signal the platform already hands over.
//!
//! What is asserted is this side of that seam: the flag lands, and coming back
//! from offline is a *nudge* rather than only a flag change — the engine
//! reconnects with backoff on its own and works with no signal at all, so the
//! whole value of knowing is promptness.

use postio_ffi::{Session, SessionOptions};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn the_flag_lands_where_the_reader_can_see_it() {
    // `Absent::Offline` versus `Absent::Partial` is the whole reason this
    // setter exists (#591): a body that is not coming reads differently from
    // one still on its way.
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
    // The promptness half. Without it, waking a laptop waits out whatever
    // backoff step the engine had reached, which can be minutes.
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
    // `NWPathMonitor` repeats itself: an interface changing while still
    // satisfied is a fresh callback with the same answer. Reconnecting on
    // every one of those would hammer the server on a flapping connection --
    // the opposite of what backoff is for.
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
