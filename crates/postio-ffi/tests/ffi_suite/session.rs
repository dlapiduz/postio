//! The boundary's lifecycle and event drain, driven the way Swift drives it.
//!
//! Nothing here needs a display, a keyring, a D-Bus session or the network,
//! which is what lets it run on macOS and Linux alike — the point of the seam
//! being proven on the cheap platform (ADR 0019).

use std::sync::Arc;

use postio_core::bridge::{Bridge, handler_fn};
use postio_ffi::{Session, SessionOptions, UiEvent};

/// A session over an in-memory store, with no disk and no keyring.
fn session() -> Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session opens")
}

#[test]
fn an_in_memory_session_opens_without_a_store_on_disk() {
    let session = session();
    // The store is real: a session that reported success while holding
    // nothing would pass every later assertion in this file vacuously.
    assert!(
        session.is_open(),
        "the session reported open but holds no store"
    );
    session.shutdown();
}

#[test]
fn shutdown_is_idempotent() {
    // Swift's app lifecycle can deliver a termination twice — a window close
    // racing an explicit quit — and a boundary that panicked on the second
    // one would take the process down during ordinary shutdown.
    let session = session();
    session.shutdown();
    session.shutdown();
}

#[test]
fn an_emitted_event_reaches_the_drain() {
    let session = session();

    // Exactly what the engine does when a mailbox's list changes.
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
    // Tier 2 is ignorable, not silent. A variant the boundary does not model
    // yet still reaches Swift as `Other { kind }`, so an unknown event is a
    // log line on the far side rather than an event that never happened --
    // which is the difference between a frontend that is behind and one that
    // is wrong.
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
    // Swift's drain is `while let event = await session.nextEvent()`. If the
    // stream never ends, that Task never finishes and the app cannot quit.
    let session = session();
    session.shutdown();
    assert!(
        session.next_event_blocking().is_none(),
        "the drain kept yielding after shutdown, so Swift's loop would never end"
    );
}

#[test]
fn a_bridge_supplied_session_uses_the_caller_s_runtime() {
    // `postio-app` builds its own `Bridge` and hands the parts to `Wiring`.
    // The macOS app does the same through the boundary, so the constructor
    // has to accept an existing runtime rather than insisting on its own.
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let session = Session::open(SessionOptions::in_memory_on(
        bridge.handle(),
        bridge.commands(),
    ))
    .expect("a session over a caller-supplied bridge");
    assert!(session.is_open());
    session.shutdown();
}
