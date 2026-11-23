use std::sync::Arc;

use postio_core::bridge::{Bridge, handler_fn};
use postio_ffi::{Session, SessionOptions, UiEvent};

fn session() -> Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session opens")
}

#[test]
fn an_in_memory_session_opens_without_a_store_on_disk() {
    let session = session();
    // A session that reported success while holding nothing would pass every later assertion
    // vacuously.
    assert!(
        session.is_open(),
        "the session reported open but holds no store"
    );
    session.shutdown();
}

#[test]
fn shutdown_is_idempotent() {
    // Swift can deliver a termination twice (a window close racing a quit); a panic would take the
    // process down in ordinary shutdown.
    let session = session();
    session.shutdown();
    session.shutdown();
}

#[test]
fn an_emitted_event_reaches_the_drain() {
    let session = session();

    session.emit_for_test(postio_core::Event::MessageListChanged {
        account: 7.into(),
        mailbox: 42.into(),
    });

    let event = session
        .next_event_blocking()
        .expect("the drain delivers the event that was emitted");
    match event {
        UiEvent::MessageListChanged { account, mailbox } => {
            assert_eq!(account, 7);
            assert_eq!(mailbox, 42);
        }
        other => panic!("expected MessageListChanged, got {other:?}"),
    }
    session.shutdown();
}

#[test]
fn an_unmodelled_event_becomes_other_rather_than_being_dropped() {
    // Tier 2 is ignorable, not silent: an unmodelled event reaches Swift as `Other { kind }`, a log
    // line rather than a lost event.
    let session = session();
    session.emit_for_test(postio_core::Event::MailboxesChanged { account: 1.into() });
    let event = session.next_event_blocking().expect("an event arrives");
    assert!(
        matches!(
            event,
            UiEvent::MailboxesChanged { .. } | UiEvent::Other { .. }
        ),
        "an unmodelled event was dropped instead of surfacing: {event:?}"
    );
    session.shutdown();
}

#[test]
fn the_drain_ends_when_the_session_shuts_down() {
    // Swift drains with `while let event = await session.nextEvent()`; a stream that never ends
    // keeps the app from quitting.
    let session = session();
    session.shutdown();
    assert!(
        session.next_event_blocking().is_none(),
        "the drain kept yielding after shutdown, so Swift's loop would never end"
    );
}

#[test]
fn a_bridge_supplied_session_uses_the_caller_s_runtime() {
    // The macOS app builds its own `Bridge` as the classic one does, so the constructor must accept
    // an existing runtime.
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let session = Session::open(SessionOptions::in_memory_on(
        bridge.handle(),
        bridge.commands(),
    ))
    .expect("a session over a caller-supplied bridge");
    assert!(session.is_open());
    session.shutdown();
}
