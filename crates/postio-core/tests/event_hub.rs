//! The event hub: N producers in, N subscribers out (ADR 0013).
//!
//! Before this existed there was one `EventStream` per channel and exactly one
//! reader of it, so a tracked caller and the window could not both listen —
//! ADR 0002 ended on that constraint and ADR 0010 then decided MCP is a second
//! frontend beside the running window, which is the configuration the
//! constraint forbids.
//!
//! What these tests hold to, in ADR 0013's own terms:
//!
//! - **Delivery is total per subscriber** (Q2). `async_channel`'s receiver is
//!   work-stealing, so the wrong shape here does not drop events visibly — it
//!   splits them between subscribers, and each one holds state that is
//!   silently wrong. Every fan-out test therefore asserts on *both* streams,
//!   never on one and a count.
//! - **A late subscriber starts at now** (Q4). The hub keeps no history.
//! - **The hub filters nothing** (Q3); correlation stays a per-subscriber
//!   question, answered with `is_from`.
//! - **Unbounded, with a watermark that warns by label** (Q2).
//!
//! Not `#[tokio::test]`, for the reason `bridge.rs` gives: the frontend drives
//! this from a plain thread that owns no runtime.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use postio_core::bridge::{Bridge, EventHub, EventSink, EventStream, handler_fn};
use postio_core::dispatch::Dispatcher;
use postio_core::invocation::InvocationOutcome;
use postio_core::{Command, CommandId, Event, MessageTarget};

/// A bus whose `Archive` reports itself, so a tracked send has something to
/// be correlated with. `InvocationFinished` is the [`Dispatcher`]'s doing,
/// not a bare handler's — which is why this is not `echo()`.
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
        std::thread::sleep(Duration::from_millis(2));
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

/// Drain everything queued right now, as the window's loop would.
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

    // Both, in full. A work-stealing receiver would split these two events
    // one each and every assertion on a single stream would still pass.
    assert_eq!(drained(&window), ["archive", "undo"]);
    assert_eq!(drained(&mcp), ["archive", "undo"]);
}

#[test]
fn several_producers_all_reach_every_subscriber() {
    let hub = EventHub::new();
    let window = hub.subscribe("window");
    let mcp = hub.subscribe("mcp");

    // The application's two: the bus's handlers and the sync engine.
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

    // ADR 0013 Q4: the hub keeps no history, so the store is where the past
    // lives. A replay buffer here would be a second unbounded copy of recent
    // mailbox activity that no consumer asked for.
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
    // Pruning happens under the write lock a subscribe already takes, so the
    // dead queue is gone by the time a third consumer joins.
    let _third = hub.subscribe("third");

    assert_eq!(hub.subscribers(), 2, "{hub:?}");
    // The count alone would pass on a table that still holds the dead queue,
    // because it filters. `Debug` is the table itself -- labels and depths,
    // which is also the audit hook ADR 0013 Q3 asks the label to be.
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
    // A spawned task's sink, kept long after the handler returned.
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

/// A writer every `tracing` line lands in, so a test can read them back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("not poisoned")).into_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn a_subscriber_that_never_drains_becomes_a_line_in_the_log() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();

    // Thread-local: every emit below happens on this thread, so nothing needs
    // the global default, and two test binaries cannot race over it.
    tracing::subscriber::with_default(subscriber, || {
        let hub = EventHub::new();
        let draining = hub.subscribe("window");
        let _wedged = hub.subscribe("mcp");
        let sink = hub.sink();

        for n in 0..(postio_core::bridge::SUBSCRIBER_DEPTH_WATERMARK + 8) {
            sink.emit(completed(&n.to_string()));
            // The one that is doing its job stays shallow, and must not be
            // named: a warning that fires for a healthy subscriber is a
            // warning nobody reads.
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
    // Never content, at any level -- the same rule the rest of the tree lives
    // by. Only a label and counts may appear.
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

    // The engine's half of the application: not a command handler, so it
    // holds a sink of its own rather than being handed one.
    let engine: EventSink = hub.sink();
    engine.emit(completed("mailbox-changed"));

    let invocation = bridge
        .commands()
        .send_tracked(Command::Archive {
            target: MessageTarget::Selection,
        })
        .expect("running");
    bridge.shutdown();

    // Acceptance: a tracked send's InvocationFinished reaches *every*
    // subscriber, and each one can filter on its own.
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
                    // The engine's event carries no origin, so a caller
                    // filtering on its own invocation correctly ignores it.
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
    // The whole compatibility claim of ADR 0013 in one test: `Bridge::new`
    // keeps its signature and its behaviour, hub or no hub, so postio-gtk and
    // every existing test compile and pass unchanged.
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
