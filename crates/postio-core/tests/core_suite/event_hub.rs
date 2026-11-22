use std::time::{Duration, Instant};

use postio_core::bridge::{Bridge, EventHub, EventSink, EventStream, handler_fn};
use postio_core::dispatch::Dispatcher;
use postio_core::invocation::InvocationOutcome;
use postio_core::{Command, CommandId, Event, MessageTarget};

fn archiving_bus() -> Dispatcher {
    Dispatcher::builder()
        .on(CommandId::Archive, |invocation| async move {
            invocation.emit(Event::ActionCompleted {
                description: "archive".to_owned(),
                undoable: true,
            });
            Ok(())
        })
        .build()
}

const PATIENCE: Duration = Duration::from_secs(5);

fn next_event(events: &EventStream) -> Event {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(event) = events.try_next() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "no event arrived within {PATIENCE:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn description(event: &Event) -> String {
    match event {
        Event::ActionCompleted { description, .. } => description.clone(),
        other => panic!("expected ActionCompleted, got {other:?}"),
    }
}

fn completed(description: &str) -> Event {
    Event::ActionCompleted {
        description: description.to_owned(),
        undoable: false,
    }
}

fn drained(events: &EventStream) -> Vec<String> {
    let mut seen = Vec::new();
    while let Some(event) = events.try_next() {
        seen.push(description(&event));
    }
    seen
}

#[test]
fn every_subscriber_receives_every_event() {
    let hub = EventHub::new();
    let window = hub.subscribe("window");
    let mcp = hub.subscribe("mcp");
    let sink = hub.sink();

    sink.emit(completed("archive"));
    sink.emit(completed("undo"));

    // Both in full: a work-stealing receiver would split the events and any single-stream assertion
    // would still pass.
    assert_eq!(drained(&window), ["archive", "undo"]);
    assert_eq!(drained(&mcp), ["archive", "undo"]);
}

#[test]
fn several_producers_all_reach_every_subscriber() {
    let hub = EventHub::new();
    let window = hub.subscribe("window");
    let mcp = hub.subscribe("mcp");

    let bus = hub.sink();
    let engine = hub.sink();

    bus.emit(completed("archive"));
    engine.emit(completed("mailbox-changed"));

    let seen = drained(&window);
    assert!(seen.contains(&"archive".to_owned()), "{seen:?}");
    assert!(seen.contains(&"mailbox-changed".to_owned()), "{seen:?}");
    assert_eq!(drained(&mcp), seen);
}

#[test]
fn per_producer_order_is_preserved_for_every_subscriber() {
    let hub = EventHub::new();
    let first = hub.subscribe("first");
    let second = hub.subscribe("second");
    let sink = hub.sink();

    let expected: Vec<String> = (0..64).map(|n| n.to_string()).collect();
    for description in &expected {
        sink.emit(completed(description));
    }

    assert_eq!(drained(&first), expected);
    assert_eq!(drained(&second), expected);
}

#[test]
fn a_late_subscriber_starts_at_now() {
    let hub = EventHub::new();
    let early = hub.subscribe("early");
    let sink = hub.sink();

    sink.emit(completed("before"));
    let late = hub.subscribe("late");
    sink.emit(completed("after"));

    assert_eq!(drained(&early), ["before", "after"]);
    assert_eq!(drained(&late), ["after"]);
}

#[test]
fn a_dropped_subscription_stops_costing_anything() {
    let hub = EventHub::new();
    let kept = hub.subscribe("kept");
    let dropped = hub.subscribe("dropped");
    let sink = hub.sink();

    drop(dropped);
    sink.emit(completed("archive"));
    let _third = hub.subscribe("third");

    assert_eq!(hub.subscribers(), 2, "{hub:?}");
    assert!(
        !format!("{hub:?}").contains("dropped"),
        "the dead queue is still on the emit path: {hub:?}"
    );
    assert_eq!(drained(&kept), ["archive"]);
}

#[test]
fn a_sink_with_nobody_listening_says_so() {
    let hub = EventHub::new();
    let sink = hub.sink();
    assert!(sink.is_closed(), "no subscriber has joined yet");

    let window = hub.subscribe("window");
    assert!(!sink.is_closed());
    assert!(sink.emit(completed("archive")));

    drop(window);
    assert!(sink.is_closed());
    assert!(
        !sink.emit(completed("undo")),
        "emit reports that nothing took the event"
    );
}

#[test]
fn origin_tagging_survives_the_hub() {
    let hub = EventHub::new();
    let window = hub.subscribe("window");
    let mcp = hub.subscribe("mcp");

    let invocation = postio_core::invocation::InvocationId::next();
    let tagged = hub.sink().with_origin(invocation);
    let inherited = tagged.clone();
    hub.sink().emit(completed("untagged"));
    inherited.emit(completed("tagged"));

    for events in [&window, &mcp] {
        let first = events.try_next_tracked().expect("the untagged event");
        assert_eq!(first.origin, None);
        assert!(!first.is_from(invocation));

        let second = events.try_next_tracked().expect("the tagged event");
        assert_eq!(second.origin, Some(invocation));
        assert!(second.is_from(invocation));
    }
}

use postio_test_support::logs::Captured;

#[test]
fn a_subscriber_that_never_drains_becomes_a_line_in_the_log() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        let hub = EventHub::new();
        let draining = hub.subscribe("window");
        let _wedged = hub.subscribe("mcp");
        let sink = hub.sink();

        for n in 0..(postio_core::bridge::SUBSCRIBER_DEPTH_WATERMARK + 8) {
            sink.emit(completed(&n.to_string()));
            let _ = draining.try_next();
        }
    });

    let log = captured.text();
    assert!(
        log.contains("mcp"),
        "the wedged subscriber is not named in the log: {log:?}"
    );
    assert!(
        !log.contains("window"),
        "the draining subscriber was warned about too: {log:?}"
    );
    assert_eq!(
        log.lines().count(),
        1,
        "the watermark warned more than once for one crossing: {log:?}"
    );
    // Never content, at any level: only a label and counts may appear.
    assert!(
        !log.contains("archive") && !log.contains("ActionCompleted"),
        "the warning carried event content: {log:?}"
    );
}

#[test]
fn a_bridge_can_be_built_on_a_hub_the_caller_owns() {
    let hub = EventHub::new();
    let window = hub.subscribe("window");
    let caller = hub.subscribe("caller");

    let bridge = Bridge::builder()
        .worker_threads(1)
        .build_with_events(archiving_bus(), hub.sink())
        .expect("the runtime starts");

    let engine: EventSink = hub.sink();
    engine.emit(completed("mailbox-changed"));

    let invocation = bridge
        .commands()
        .send_tracked(Command::Archive {
            target: MessageTarget::Selection,
        })
        .expect("running");
    bridge.shutdown();

    for events in [&window, &caller] {
        let mut finished = None;
        let mut saw_engine = false;
        while let Some(envelope) = events.try_next_tracked() {
            match &envelope.event {
                Event::InvocationFinished {
                    invocation: id,
                    outcome,
                } => {
                    assert!(envelope.is_from(invocation));
                    assert_eq!(*id, invocation);
                    finished = Some(outcome.clone());
                }
                Event::ActionCompleted { description, .. } if description == "mailbox-changed" => {
                    assert!(!envelope.is_from(invocation));
                    saw_engine = true;
                }
                _ => {}
            }
        }
        assert!(saw_engine, "a producer that is not a handler went missing");
        assert!(
            matches!(finished, Some(InvocationOutcome::Completed)),
            "no InvocationFinished for this subscriber: {finished:?}"
        );
    }
}

#[test]
fn bridge_new_still_hands_back_one_working_stream() {
    let (bridge, events) = Bridge::new(echo()).expect("the runtime starts");
    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    assert_eq!(description(&next_event(&events)), "refresh");
}

fn echo() -> impl postio_core::bridge::CommandHandler {
    handler_fn(|command: Command, events: EventSink| async move {
        events.emit(Event::ActionCompleted {
            description: command.id().to_string(),
            undoable: false,
        });
    })
}
