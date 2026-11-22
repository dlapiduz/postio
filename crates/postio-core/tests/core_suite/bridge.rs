use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use postio_core::bridge::{Bridge, EventStream, handler_fn};
use postio_core::{Command, Event, MessageTarget};

fn next_event(events: &EventStream) -> Event {
    let mut taken = None;
    postio_test_support::wait_until("an event to arrive on the bridge", || {
        taken = events.try_next();
        taken.is_some()
    });
    taken.expect("the wait only returns once an event was taken")
}

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
    let (bridge, events) = Bridge::new(echo()).expect("the runtime starts");
    let commands = bridge.commands();

    let sent = [
        Command::Archive {
            target: MessageTarget::Selection,
        },
        Command::Undo,
        Command::Refresh,
        Command::ToggleFold,
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
    let (bridge, events) = Bridge::new(handler_fn(|command: Command, events| async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        events.emit(Event::ActionCompleted {
            description: command.id().to_string(),
            undoable: false,
        });
    }))
    .expect("the runtime starts");

    for _ in 0..8 {
        bridge.commands().send(Command::Refresh).expect("running");
    }

    // Asserted as ordering, not a stopwatch (#1068): each handler sleeps 250ms, so a `send` that
    // waited would let an event through before the next send.
    assert!(
        events.try_next().is_none(),
        "a handler ran to completion while the caller was still sending, so \
         `send` is waiting for the backend the UI thread must never await"
    );

    assert_eq!(description(&next_event(&events)), "refresh");
}

#[test]
fn work_offloaded_onto_the_runtime_does_not_delay_the_next_command() {
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
        .send(Command::ToggleFold)
        .expect("running");

    let start = Instant::now();
    assert_eq!(description(&next_event(&events)), "refresh");
    assert_eq!(description(&next_event(&events)), "toggle_fold");
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
    // #817: a merely slow handler must not turn a scheduling delay into a torn-down caller; the
    // second
    // `shutdown_timeout` window gives the pump another chance.
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
    let (bridge, _events) = Bridge::new(handler_fn(|_command: Command, _events| async move {
        // POSTIO-NOT-A-WAIT: the stuck task is the subject; the test returns without it.
        tokio::spawn(async { tokio::time::sleep(Duration::from_secs(60)).await });
    }))
    .expect("the runtime starts");

    bridge.commands().send(Command::Refresh).expect("running");
    let start = Instant::now();
    bridge.shutdown();
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

/// Regression for 0.1.0, which enabled only the timer: any socket opened on this runtime panicked
/// "IO is disabled". Binds loopback only, so no network is touched.
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
