//! Priority, not arrival order, decides who writes next (#425). `busy_timeout` is a retry loop with
//! no ordering, so "interactive first" has to be arranged above SQLite.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use postio_storage::{WriteGate, WritePriority, test_support};

const ENOUGH_TO_BLOCK: Duration = Duration::from_millis(50);

/// Waits on the gate's own observable to establish arrival order without a sleep; the yield keeps
/// it safe on a single-worker runtime.
async fn until_interactive_is_waiting(gate: &WriteGate) {
    while !gate.interactive_is_waiting() {
        tokio::task::yield_now().await;
    }
}

async fn gate() -> WriteGate {
    test_support::memory().await.write_gate().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interactive_writer_goes_first_even_though_it_asked_second() {
    let gate = gate().await;
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    let blocking = gate.acquire(WritePriority::Interactive).await;

    let background = tokio::spawn({
        let gate = gate.clone();
        let order = Arc::clone(&order);
        async move {
            let _permit = gate.acquire(WritePriority::Background).await;
            order.lock().unwrap().push("background");
        }
    });
    tokio::time::sleep(ENOUGH_TO_BLOCK).await;

    let interactive = tokio::spawn({
        let gate = gate.clone();
        let order = Arc::clone(&order);
        async move {
            let _permit = gate.acquire(WritePriority::Interactive).await;
            order.lock().unwrap().push("interactive");
        }
    });
    until_interactive_is_waiting(&gate).await;

    drop(blocking);
    interactive.await.expect("the interactive writer finishes");
    background.await.expect("the background writer finishes");

    assert_eq!(
        *order.lock().unwrap(),
        vec!["interactive", "background"],
        "the backfill was already queued and took the lock anyway. Arrival \
         order is what SQLite's own busy_timeout gives, and giving it is the \
         bug — a person's write has to overtake bulk work that got there \
         first, or a first sync locks them out for as long as it runs."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_background_writer_waits_for_a_queued_interactive_one() {
    // A background writer must not begin while an interactive one waits: yielding after taking
    // SQLite's lock is too late.
    let gate = gate().await;

    let blocking = gate.acquire(WritePriority::Background).await;

    let waiting = tokio::spawn({
        let gate = gate.clone();
        async move {
            let permit = gate.acquire(WritePriority::Interactive).await;
            tokio::time::sleep(ENOUGH_TO_BLOCK).await;
            drop(permit);
        }
    });
    until_interactive_is_waiting(&gate).await;

    drop(blocking);

    let started = std::time::Instant::now();
    let _permit = gate.acquire(WritePriority::Background).await;
    let waited = started.elapsed();

    assert!(
        !gate.interactive_is_waiting(),
        "a background writer was granted the lock with an interactive writer \
         still queued behind it"
    );
    assert!(
        waited >= ENOUGH_TO_BLOCK / 2,
        "the background writer was granted the lock immediately ({waited:?}), \
         so it did not wait for the interactive writer that was already queued"
    );
    waiting.await.expect("the interactive writer finishes");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_interactive_writers_do_not_hold_the_lock_at_once() {
    // The gate is a lock as well as a queue: two permits at once would put a sync batch and a
    // keystroke both inside `BEGIN IMMEDIATE` (#79).
    let gate = gate().await;
    let holders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let tasks: Vec<_> = (0..8)
        .map(|n| {
            let gate = gate.clone();
            let holders = Arc::clone(&holders);
            let peak = Arc::clone(&peak);
            let priority = if n % 2 == 0 {
                WritePriority::Interactive
            } else {
                WritePriority::Background
            };
            tokio::spawn(async move {
                for _ in 0..50 {
                    let _permit = gate.acquire(priority).await;
                    let now = holders.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    holders.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.expect("a writer finishes");
    }

    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "two writers held the gate at the same time"
    );
}

/// The gate wakes every waiter on release, which stopped being harmless when the body backfill took
/// a permit per body: a folder sync's single acquisition raced a continuous stream and starved. The
/// loop is the backfill's shape.
#[tokio::test(flavor = "multi_thread")]
async fn a_lone_writer_is_not_starved_by_a_busy_one() {
    let gate = gate().await;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let busy = tokio::spawn({
        let gate = gate.clone();
        let stop = Arc::clone(&stop);
        async move {
            let mut taken = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let permit = gate.acquire(WritePriority::Background).await;
                taken += 1;
                tokio::task::yield_now().await;
                drop(permit);
            }
            taken
        }
    });

    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    let lone = tokio::time::timeout(Duration::from_secs(10), async {
        let _permit = gate.acquire(WritePriority::Background).await;
    })
    .await;

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let taken = busy.await.expect("the busy writer");

    assert!(
        lone.is_ok(),
        "a writer that asked once never got the gate while a rival took it \
         {taken} times. A folder sync starves behind the body backfill this \
         way, and one starved pass stops every later sync wave."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gate_records_what_it_did_for_the_load_gate_to_read() {
    // `gate_log` is the instrument behind the interaction-under-load gate in `postio-sync`, which
    // needs the gate to say in order who asked and who was served.
    use postio_storage::test_support::gate_log::{self, Event};
    let gate = gate().await;
    gate_log::reset();

    let background = gate.acquire(WritePriority::Background).await;
    let interactive = tokio::spawn({
        let gate = gate.clone();
        async move { gate.acquire(WritePriority::Interactive).await }
    });
    until_interactive_is_waiting(&gate).await;
    drop(background);
    let _permit = interactive.await.expect("the interactive writer is served");

    assert_eq!(
        gate_log::events(),
        [
            Event::Requested(WritePriority::Background),
            Event::Granted(WritePriority::Background),
            Event::Requested(WritePriority::Interactive),
            Event::Granted(WritePriority::Interactive),
        ],
        "every request and every grant, in the order they happened"
    );
    assert_eq!(
        gate_log::background_grants_while_interactive_waited(0),
        Some(0),
        "and the reading the load gate takes off it"
    );
}
