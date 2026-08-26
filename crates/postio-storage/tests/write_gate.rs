//! The write gate hands the lock to a person before a backfill. #425.
//!
//! `postio-session/tests/interactive_write.rs` is the end-to-end claim — an
//! archive keystroke does not queue behind a first sync. This is the property
//! underneath it, stated where it can be asserted without a mail server: that
//! priority, not arrival order, decides who writes next.
//!
//! That is the whole reason the gate exists. SQLite's own answer to two
//! writers is `busy_timeout`, which is a retry loop with no ordering in it at
//! all — so "the interactive writer goes first" is not something the database
//! can be asked for, and has to be arranged above it.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use postio_storage::{WriteGate, WritePriority, test_support};

/// Long enough for a thread that has said it is about to block to actually be
/// blocked. Not a performance assertion — it only establishes *arrival order*,
/// which is the thing these tests need to be able to set up. Both tests would
/// still be correct if it were longer; they would merely be slower.
const ENOUGH_TO_BLOCK: Duration = Duration::from_millis(50);

fn gate() -> WriteGate {
    // Through a real database, because that is how every caller reaches one
    // and it is worth knowing the wiring is there.
    test_support::memory().write_gate().clone()
}

#[test]
fn an_interactive_writer_goes_first_even_though_it_asked_second() {
    let gate = gate();
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    // Nobody writes until this is dropped, so both threads below are
    // definitely queued rather than racing to be first.
    let blocking = gate.acquire(WritePriority::Interactive);

    // The backfill asks first...
    let (announced, arrived) = mpsc::channel();
    let background = std::thread::spawn({
        let gate = gate.clone();
        let order = Arc::clone(&order);
        move || {
            announced.send(()).expect("the test is listening");
            let _permit = gate.acquire(WritePriority::Background);
            order.lock().unwrap().push("background");
        }
    });
    arrived.recv().expect("the background thread starts");
    std::thread::sleep(ENOUGH_TO_BLOCK);

    // ...and the keystroke asks second.
    let interactive = std::thread::spawn({
        let gate = gate.clone();
        let order = Arc::clone(&order);
        move || {
            let _permit = gate.acquire(WritePriority::Interactive);
            order.lock().unwrap().push("interactive");
        }
    });
    // Observable rather than slept on: the interactive writer counts itself as
    // waiting before it blocks, which is exactly what the background writer
    // has to be able to see.
    while !gate.interactive_is_waiting() {
        std::hint::spin_loop();
    }

    drop(blocking);
    interactive.join().expect("the interactive writer finishes");
    background.join().expect("the background writer finishes");

    assert_eq!(
        *order.lock().unwrap(),
        vec!["interactive", "background"],
        "the backfill was already queued and took the lock anyway. Arrival \
         order is what SQLite's own busy_timeout gives, and giving it is the \
         bug — a person's write has to overtake bulk work that got there \
         first, or a first sync locks them out for as long as it runs."
    );
}

#[test]
fn a_background_writer_waits_for_a_queued_interactive_one() {
    // The same property from the other side, and the one that actually bounds
    // the wait: a background writer must not *begin* while an interactive
    // writer is waiting. Beginning and then yielding would be yielding after
    // taking SQLite's lock, which is too late to help.
    let gate = gate();

    let blocking = gate.acquire(WritePriority::Background);

    let (announced, arrived) = mpsc::channel();
    let waiting = std::thread::spawn({
        let gate = gate.clone();
        move || {
            announced.send(()).expect("the test is listening");
            let permit = gate.acquire(WritePriority::Interactive);
            // Held, so the background attempt below has something to fail
            // against rather than a lock that is merely free.
            std::thread::sleep(ENOUGH_TO_BLOCK);
            drop(permit);
        }
    });
    arrived.recv().expect("the interactive thread starts");
    while !gate.interactive_is_waiting() {
        std::hint::spin_loop();
    }

    drop(blocking);

    // With an interactive writer queued, this must not be granted until that
    // one has come and gone.
    let started = std::time::Instant::now();
    let _permit = gate.acquire(WritePriority::Background);
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
    waiting.join().expect("the interactive writer finishes");
}

#[test]
fn two_interactive_writers_do_not_hold_the_lock_at_once() {
    // The gate is a lock as well as a queue: whatever the priorities, exactly
    // one permit is outstanding at a time. Without this the sync batch and a
    // keystroke could both be inside `BEGIN IMMEDIATE`, which is the
    // SQLITE_BUSY that #79 was.
    let gate = gate();
    let holders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let threads: Vec<_> = (0..8)
        .map(|n| {
            let gate = gate.clone();
            let holders = Arc::clone(&holders);
            let peak = Arc::clone(&peak);
            let priority = if n % 2 == 0 {
                WritePriority::Interactive
            } else {
                WritePriority::Background
            };
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let _permit = gate.acquire(priority);
                    let now = holders.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
                    std::thread::yield_now();
                    holders.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a writer finishes");
    }

    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "two writers held the gate at the same time"
    );
}
