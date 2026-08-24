//! Correlating one invocation with the events it caused.
//!
//! `send` is fire-and-forget, and that is right for GTK: a repaint does not
//! care which keystroke caused it. A programmatic caller — MCP, AI, a future
//! CLI — needs the opposite. It has to know whether *its* archive succeeded,
//! and the sync engine emits `MessagesChanged` constantly for reasons of its
//! own, so watching the global stream and matching by shape and timing is
//! unreliable the moment two commands are in flight.
//!
//! Everything here is the tracked half of the bus. The untracked half must
//! keep behaving exactly as it did — that is what
//! [`an_untracked_command_carries_no_origin`] and
//! [`an_untracked_command_announces_no_completion`] hold in place.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_core::bridge::{Bridge, EventStream};
use postio_core::dispatch::{CommandError, Dispatcher};
use postio_core::invocation::{EventEnvelope, InvocationId, InvocationOutcome};
use postio_core::{Command, CommandId, Event};
use postio_model::MessageId;

fn drain(events: &EventStream) -> Vec<EventEnvelope> {
    std::iter::from_fn(|| events.try_next_tracked()).collect()
}

fn message(id: i64) -> MessageId {
    MessageId::new(id)
}

/// A bus where `Archive` reports which messages it touched and `Refresh` is
/// the engine noise every real stream is full of.
fn noisy_bus() -> Dispatcher {
    Dispatcher::builder()
        .on(CommandId::Archive, |invocation| async move {
            invocation.emit(Event::MessagesChanged {
                messages: vec![message(1)],
            });
            Ok(())
        })
        .on(CommandId::Refresh, |invocation| async move {
            invocation.emit(Event::MessagesChanged {
                messages: vec![message(99)],
            });
            Ok(())
        })
        .build()
}

// -- The acceptance criterion ------------------------------------------------

#[test]
fn a_caller_can_pick_its_own_events_out_of_an_unrelated_stream() {
    let (bridge, events) = Bridge::new(noisy_bus()).expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");
    let mine = bridge
        .commands()
        .send_tracked(Command::default_for(CommandId::Archive))
        .expect("running");
    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    let all = drain(&events);
    assert!(
        all.len() > 3,
        "the unrelated commands must really be on the stream: {all:?}"
    );

    let ours: Vec<&Event> = all
        .iter()
        .filter(|envelope| envelope.is_from(mine))
        .map(|envelope| &envelope.event)
        .collect();

    assert!(
        matches!(
            ours.as_slice(),
            [
                Event::MessagesChanged { messages },
                Event::InvocationFinished {
                    outcome: InvocationOutcome::Completed,
                    ..
                }
            ] if messages == &vec![message(1)]
        ),
        "expected only this invocation's own events, got {ours:?}"
    );
}

#[test]
fn two_invocations_in_flight_stay_told_apart() {
    let (bridge, events) = Bridge::new(noisy_bus()).expect("the runtime starts");

    let first = bridge
        .commands()
        .send_tracked(Command::default_for(CommandId::Archive))
        .expect("running");
    let second = bridge
        .commands()
        .send_tracked(Command::default_for(CommandId::Archive))
        .expect("running");
    bridge.shutdown();

    assert_ne!(first, second, "each invocation gets an id of its own");

    let all = drain(&events);
    let count = |id: InvocationId| all.iter().filter(|e| e.is_from(id)).count();
    assert_eq!(count(first), 2, "one change plus one completion: {all:?}");
    assert_eq!(count(second), 2, "one change plus one completion: {all:?}");
}

// -- The untracked half is unchanged -----------------------------------------

#[test]
fn an_untracked_command_carries_no_origin() {
    let (bridge, events) = Bridge::new(noisy_bus()).expect("the runtime starts");
    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    let all = drain(&events);
    assert!(
        all.iter().all(|envelope| envelope.origin.is_none()),
        "fire-and-forget stays anonymous: {all:?}"
    );
}

#[test]
fn an_untracked_command_announces_no_completion() {
    // The GTK frontend must not start seeing a new event per keystroke.
    let (bridge, events) = Bridge::new(noisy_bus()).expect("the runtime starts");
    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    let all = drain(&events);
    assert!(
        !all.iter()
            .any(|envelope| matches!(envelope.event, Event::InvocationFinished { .. })),
        "no completion event without a tracked caller: {all:?}"
    );
}

#[test]
fn the_plain_stream_still_yields_plain_events() {
    // Every existing consumer reads `next`/`try_next` and must keep compiling
    // and behaving identically.
    let (bridge, events) = Bridge::new(noisy_bus()).expect("the runtime starts");
    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    let first: Option<Event> = events.try_next();
    assert!(matches!(first, Some(Event::MessagesChanged { .. })));
}

// -- Every ending is reported ------------------------------------------------

#[test]
fn a_rejection_finishes_the_invocation() {
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Undo, |_| async {
            Err(CommandError::rejected("nothing to undo"))
        })
        .build();
    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::Undo)
        .expect("running");
    bridge.shutdown();

    let ours: Vec<Event> = drain(&events)
        .into_iter()
        .filter(|envelope| envelope.is_from(mine))
        .map(|envelope| envelope.event)
        .collect();

    assert!(
        matches!(
            ours.as_slice(),
            [
                Event::CommandRejected { .. },
                Event::InvocationFinished {
                    outcome: InvocationOutcome::Rejected { reason },
                    ..
                }
            ] if reason == "nothing to undo"
        ),
        "{ours:?}"
    );
}

#[test]
fn a_failure_finishes_the_invocation() {
    let dispatcher = Dispatcher::builder()
        .on(CommandId::SaveDraft, |_| async {
            Err(CommandError::failed("the disk is full"))
        })
        .build();
    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::default_for(CommandId::SaveDraft))
        .expect("running");
    bridge.shutdown();

    let ours: Vec<Event> = drain(&events)
        .into_iter()
        .filter(|envelope| envelope.is_from(mine))
        .map(|envelope| envelope.event)
        .collect();

    assert!(
        matches!(
            ours.as_slice(),
            [
                Event::Error { .. },
                Event::InvocationFinished {
                    outcome: InvocationOutcome::Failed { message },
                    ..
                }
            ] if message == "the disk is full"
        ),
        "{ours:?}"
    );
}

#[test]
fn a_panicking_handler_still_finishes_the_invocation() {
    // The bug this exists to catch: a caller that awaits a completion it will
    // never get waits forever. A handler that panics is a bug in the handler;
    // it must not become a hung caller.
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Archive, |_| async {
            panic!("a handler bug");
        })
        .build();
    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::default_for(CommandId::Archive))
        .expect("running");
    bridge.shutdown();

    let ours: Vec<Event> = drain(&events)
        .into_iter()
        .filter(|envelope| envelope.is_from(mine))
        .map(|envelope| envelope.event)
        .collect();

    assert!(
        ours.iter().any(|event| matches!(
            event,
            Event::InvocationFinished {
                outcome: InvocationOutcome::Failed { .. },
                ..
            }
        )),
        "a panic must still end the invocation: {ours:?}"
    );
}

#[test]
fn a_command_with_no_handler_still_finishes_the_invocation() {
    let (bridge, events) = Bridge::new(Dispatcher::builder().build()).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::Undo)
        .expect("running");
    bridge.shutdown();

    let ours: Vec<Event> = drain(&events)
        .into_iter()
        .filter(|envelope| envelope.is_from(mine))
        .map(|envelope| envelope.event)
        .collect();

    assert!(
        matches!(
            ours.as_slice(),
            [
                Event::CommandRejected { .. },
                Event::InvocationFinished {
                    outcome: InvocationOutcome::Rejected { .. },
                    ..
                }
            ]
        ),
        "an unwired command must not strand its caller: {ours:?}"
    );
}

// -- What a handler can see and hand on --------------------------------------

#[test]
fn a_handler_can_read_the_id_it_was_invoked_under() {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let recorder = Arc::clone(&seen);
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Archive, move |invocation| {
            let recorder = Arc::clone(&recorder);
            async move {
                *recorder.lock().unwrap() = invocation.invocation_id();
                Ok(())
            }
        })
        .build();

    let (bridge, _events) = Bridge::new(dispatcher).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::default_for(CommandId::Archive))
        .expect("running");
    bridge.shutdown();

    assert_eq!(*seen.lock().unwrap(), Some(mine));
}

#[test]
fn work_a_handler_spawns_keeps_reporting_under_the_same_id() {
    // A body fetch, a send, a resync: the handler returns and the work keeps
    // going. Those events answer the invocation that started them.
    let done = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&done);
    let dispatcher = Dispatcher::builder()
        .on(CommandId::Refresh, move |invocation| {
            let counter = Arc::clone(&counter);
            async move {
                let events = invocation.events();
                tokio::spawn(async move {
                    events.emit(Event::MessageListChanged {
                        mailbox: postio_model::MailboxId::new(7),
                    });
                    counter.fetch_add(1, Ordering::SeqCst);
                })
                .await
                .expect("the task runs");
                Ok(())
            }
        })
        .build();

    let (bridge, events) = Bridge::new(dispatcher).expect("the runtime starts");
    let mine = bridge
        .commands()
        .send_tracked(Command::Refresh)
        .expect("running");
    bridge.shutdown();

    assert_eq!(done.load(Ordering::SeqCst), 1);
    assert!(
        drain(&events).iter().any(|envelope| envelope.is_from(mine)
            && matches!(envelope.event, Event::MessageListChanged { .. })),
        "a detached task inherits the invocation it was started for"
    );
}

#[test]
fn a_completion_survives_a_serde_round_trip() {
    // Events are logged and replayed; a programmatic consumer reads them off
    // the wire.
    let event = Event::InvocationFinished {
        invocation: InvocationId::next(),
        outcome: InvocationOutcome::Rejected {
            reason: "nothing to undo".into(),
        },
    };
    let json = serde_json::to_string(&event).expect("serialises");
    assert_eq!(serde_json::from_str::<Event>(&json).expect("parses"), event);
}
