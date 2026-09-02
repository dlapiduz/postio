//! The tokio↔glib bridge, exercised the way the GTK thread uses it: from an
//! ordinary synchronous thread that owns no runtime of its own.
//!
//! These tests are deliberately *not* `#[tokio::test]`. The frontend calls
//! `send` from the GTK main loop and drains events there; if the bridge only
//! worked from inside a tokio context it would not be a bridge at all. Nothing
//! here touches the network, and every wait is bounded so a regression fails
//! the suite instead of hanging it.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use postio_core::bridge::{Bridge, EventStream, handler_fn};
use postio_core::{Command, Event, MessageTarget};

/// Wait for the next event, or fail the test rather than hang the suite.
///
/// The deadline and the backoff come from `postio_test_support`, so
/// `POSTIO_TEST_PATIENCE` reaches this the way it reaches every other wait in
/// the workspace. It used to be a `PATIENCE` constant defined here, which is
/// how the suite ended up with 171 of them.
fn next_event(events: &EventStream) -> Event {
    let mut taken = None;
    postio_test_support::wait_until("an event to arrive on the bridge", || {
        taken = events.try_next();
        taken.is_some()
    });
    taken.expect("the wait only returns once an event was taken")
}

/// An echo handler: every command becomes one `ActionCompleted` event naming it.
fn echo() -> impl postio_core::bridge::CommandHandler {
    handler_fn(|command: Command, events| async move {
        events.emit(Event::ActionCompleted {
            description: command.id().to_string(),
            undoable: false,
        });
    })
}

fn description(event: &Event) -> &str {
    match event {
        Event::ActionCompleted { description, .. } => description,
        other => panic!("expected ActionCompleted, got {other:?}"),
    }
}

#[test]
fn a_command_reaches_the_handler_and_its_event_comes_back() {
    let (bridge, events) = Bridge::new(echo()).expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");

    assert_eq!(description(&next_event(&events)), "refresh");
}

#[test]
fn commands_are_handled_in_the_order_they_were_sent() {
    // Archive-then-undo must not race: the bus is a queue, not a thread pool.
    let (bridge, events) = Bridge::new(echo()).expect("the runtime starts");
    let commands = bridge.commands();

    let sent = [
        Command::Archive {
            target: MessageTarget::Selection,
        },
        Command::Undo,
        Command::Refresh,
        Command::ToggleSidebar,
    ];
    for command in &sent {
        commands.send(command.clone()).expect("running");
    }

    let seen: Vec<String> = sent
        .iter()
        .map(|_| description(&next_event(&events)).to_owned())
        .collect();
    let expected: Vec<String> = sent.iter().map(|c| c.id().to_string()).collect();
    assert_eq!(seen, expected);
}

#[test]
fn sending_never_blocks_the_calling_thread() {
    // The UI thread must never await the backend. A handler that takes a
    // quarter of a second must not cost the caller a quarter of a second.
    let (bridge, events) = Bridge::new(handler_fn(|command: Command, events| async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        events.emit(Event::ActionCompleted {
            description: command.id().to_string(),
            undoable: false,
        });
    }))
    .expect("the runtime starts");

    let start = Instant::now();
    for _ in 0..8 {
        bridge.commands().send(Command::Refresh).expect("running");
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(50),
        "sending eight commands blocked the caller for {elapsed:?}"
    );

    // And the work really did happen, off the caller's thread.
    assert_eq!(description(&next_event(&events)), "refresh");
}

#[test]
fn work_offloaded_onto_the_runtime_does_not_delay_the_next_command() {
    // Handlers are local-first and quick; anything slow is spawned. This is
    // what keeps sequential dispatch from becoming a head-of-line stall.
    let finished = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&finished);
    let (bridge, events) = Bridge::new(handler_fn(move |command: Command, events| {
        let counter = Arc::clone(&counter);
        async move {
            if matches!(command, Command::Refresh) {
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                });
            }
            events.emit(Event::ActionCompleted {
                description: command.id().to_string(),
                undoable: false,
            });
        }
    }))
    .expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");
    bridge
        .commands()
        .send(Command::ToggleSidebar)
        .expect("running");

    let start = Instant::now();
    assert_eq!(description(&next_event(&events)), "refresh");
    assert_eq!(description(&next_event(&events)), "toggle_sidebar");
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "the spawned sleep stalled the queue"
    );
    assert_eq!(
        finished.load(Ordering::SeqCst),
        0,
        "the sleep is still running"
    );
}

#[test]
fn commands_may_be_sent_from_many_threads_at_once() {
    // Senders cross threads; nothing in the bridge may be thread-affine.
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&handled);
    let (bridge, _events) = Bridge::new(handler_fn(move |_command: Command, _events| {
        let counter = Arc::clone(&counter);
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }))
    .expect("the runtime starts");

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let commands = bridge.commands();
            std::thread::spawn(move || {
                for _ in 0..64 {
                    commands.send(Command::Refresh).expect("running");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("sender thread did not panic");
    }

    postio_test_support::wait_until("all 512 commands to reach the handler", || {
        handled.load(Ordering::SeqCst) == 8 * 64
    });
}

#[test]
fn shutdown_finishes_the_queued_work_and_joins() {
    // Quitting must not silently drop the archive the user just asked for.
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&handled);
    let (bridge, _events) = Bridge::new(handler_fn(move |_command: Command, _events| {
        let counter = Arc::clone(&counter);
        async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }))
    .expect("the runtime starts");

    for _ in 0..16 {
        bridge.commands().send(Command::Refresh).expect("running");
    }
    bridge.shutdown();

    assert_eq!(handled.load(Ordering::SeqCst), 16);
}

#[test]
fn a_handler_slower_than_the_shutdown_timeout_does_not_panic_the_caller() {
    // #817: a handler that is merely slow -- not stuck -- must not turn an
    // ordinary scheduling delay into a torn-down caller. `shutdown_timeout`
    // governs how long `stop` waits for the pump to drain, but missing that
    // window once is not proof of a hang: `Runtime::shutdown_timeout`, called
    // right after, gives the still-running pump a second, equal-length
    // chance regardless of what the first wait concluded.
    let (bridge, _events) = Bridge::builder()
        .worker_threads(1)
        .shutdown_timeout(Duration::from_millis(20))
        .build(handler_fn(|_command: Command, _events| async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }))
        .expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();
}

#[test]
fn shutdown_does_not_wait_for_detached_background_work() {
    // A stuck sync task must not hold the application open at quit.
    let (bridge, _events) = Bridge::new(handler_fn(|_command: Command, _events| async move {
        tokio::spawn(async { tokio::time::sleep(Duration::from_secs(60)).await });
    }))
    .expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");
    let start = Instant::now();
    bridge.shutdown();
    // An upper bound, not a wait: shutdown has already returned, and the
    // question is whether it returned promptly. Shares the workspace deadline
    // so a slow machine raises this too.
    assert!(
        start.elapsed() < postio_test_support::patience(),
        "shutdown hung on a detached task"
    );
}

#[test]
fn events_emitted_before_shutdown_are_still_readable_after_it() {
    let (bridge, events) = Bridge::new(echo()).expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    assert_eq!(description(&next_event(&events)), "refresh");
    // Drained, and the producer is gone: the UI loop can exit.
    assert!(events.try_next().is_none());
    postio_test_support::wait_until("the event stream to close after shutdown", || {
        events.is_closed()
    });
}

#[test]
fn sending_after_shutdown_is_an_error_not_a_panic() {
    let (bridge, _events) = Bridge::new(echo()).expect("the runtime starts");
    let commands = bridge.commands();
    bridge.shutdown();

    let error = commands
        .send(Command::Refresh)
        .expect_err("the runtime stopped");
    assert!(error.to_string().contains("stopped"), "{error}");
}

#[test]
fn emitting_to_a_frontend_that_went_away_does_not_panic() {
    // The window can close while a handler is mid-flight; that is a no-op,
    // not a crash.
    let observed = Arc::new(AtomicUsize::new(0));
    let delivered = Arc::clone(&observed);
    let (bridge, events) = Bridge::new(handler_fn(move |command: Command, events| {
        let delivered = Arc::clone(&delivered);
        async move {
            let landed = events.emit(Event::ActionCompleted {
                description: command.id().to_string(),
                undoable: false,
            });
            delivered.fetch_add(usize::from(landed), Ordering::SeqCst);
        }
    }))
    .expect("the runtime starts");

    drop(events);
    bridge.commands().send(Command::Refresh).expect("running");
    bridge.shutdown();

    assert_eq!(
        observed.load(Ordering::SeqCst),
        0,
        "there was no one to tell"
    );
}

#[test]
fn dropping_the_bridge_shuts_it_down() {
    let (bridge, events) = Bridge::new(echo()).expect("the runtime starts");
    let commands = bridge.commands();

    bridge.commands().send(Command::Refresh).expect("running");
    drop(bridge);

    assert_eq!(description(&next_event(&events)), "refresh");
    assert!(commands.send(Command::Refresh).is_err());
}

/// The runtime the bridge builds can do IO.
///
/// It could not, until `dev.postio.Postio 0.1.0` panicked on first launch the
/// moment onboarding tried to reach a server:
///
/// ```text
/// thread 'postio-core' panicked at tokio/src/net/tcp/stream.rs:164:
/// A Tokio 1.x context was found, but IO is disabled.
/// Call `enable_io` on the runtime builder to enable IO.
/// ```
///
/// `BridgeBuilder::build` called `enable_time()` and nothing else, so every
/// socket opened on this runtime panicked — which is every socket the
/// application opens, since this is the runtime the sync engine runs on.
///
/// Nothing in the default suite had ever opened one. The no-network rule means
/// the handlers under test talk to `MockBackend`, so `enable_io` was exercised
/// by exactly nothing and the gap was invisible to a green suite.
///
/// This binds a listener on loopback and connects to it. That is not network
/// access — no name is resolved and no packet leaves the machine — and it is
/// the smallest thing that proves the reactor is running.
#[test]
fn the_bridge_runtime_can_open_a_socket() {
    let handler = handler_fn(|_command: Command, events| async move {
        let outcome = async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let accept = tokio::spawn(async move { listener.accept().await });
            tokio::net::TcpStream::connect(address).await?;
            accept.await.expect("the accept task should not panic")?;
            Ok::<_, std::io::Error>(())
        }
        .await;
        events.emit(Event::ActionCompleted {
            description: match outcome {
                Ok(()) => "io ok".to_owned(),
                Err(error) => format!("io failed: {error}"),
            },
            undoable: false,
        });
    });

    let (bridge, events) = Bridge::builder()
        .worker_threads(1)
        .build(handler)
        .expect("the runtime should start");
    bridge
        .commands()
        .send(Command::Refresh)
        .expect("the bridge should be running");

    match next_event(&events) {
        Event::ActionCompleted { description, .. } => {
            assert_eq!(description, "io ok", "the bridge runtime refused a socket")
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
