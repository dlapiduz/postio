//! The pool hands out a connection ahead of background work before ever
//! looking at SQLite's write lock. #672.
//!
//! `write_gate.rs` proves the analogous property for who gets the write
//! lock next (#425) — this is the same property one layer earlier: who gets
//! a *connection* next, which every caller (a read included) needs before
//! it can even ask for the lock.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use postio_storage::Database;
use postio_storage::test_support;

/// Long enough for a thread that has said it is about to block to actually be
/// blocked. Not a performance assertion — it only establishes *arrival
/// order*, which is the thing these tests need to be able to set up. Both
/// tests would still be correct if it were longer; they would merely be
/// slower.
const ENOUGH_TO_BLOCK: Duration = Duration::from_millis(50);

/// A pool with exactly one connection, so a single held checkout is enough to
/// exhaust it and force everything else to queue.
fn pool_of_one() -> postio_storage::Pool {
    Database::open_in_memory_with(&test_support::key(), 1)
        .expect("an in-memory database")
        .pool()
        .clone()
}

#[test]
fn an_interactive_checkout_goes_first_even_though_it_asked_second() {
    let pool = pool_of_one();
    let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

    // Nobody gets a connection until this is dropped, so both threads below
    // are definitely queued rather than racing to be first.
    let blocking = pool.get().expect("the one connection");

    // The backfill asks first...
    let (announced, arrived) = mpsc::channel();
    let background = std::thread::spawn({
        let pool = pool.clone();
        let order = Arc::clone(&order);
        move || {
            announced.send(()).expect("the test is listening");
            let _connection = pool.get().expect("a connection eventually");
            order.lock().unwrap().push("background");
        }
    });
    arrived.recv().expect("the background thread starts");
    std::thread::sleep(ENOUGH_TO_BLOCK);

    // ...and the reading pane asks second.
    let interactive = std::thread::spawn({
        let pool = pool.clone();
        let order = Arc::clone(&order);
        move || {
            let _connection = pool.get_interactive().expect("a connection eventually");
            order.lock().unwrap().push("interactive");
        }
    });
    // Observable rather than slept on: the interactive caller counts itself
    // as waiting before it blocks, which is exactly what the background
    // caller has to be able to see.
    while !pool.interactive_is_waiting() {
        std::hint::spin_loop();
    }

    drop(blocking);
    interactive
        .join()
        .expect("the interactive checkout finishes");
    background.join().expect("the background checkout finishes");

    assert_eq!(
        *order.lock().unwrap(),
        vec!["interactive", "background"],
        "the backfill was already queued for a connection and got one anyway. \
         Arrival order is what a plain FIFO wait gives, and giving it is the \
         bug — a person's read has to overtake bulk work that got there \
         first, or a first sync locks the reading pane out for as long as it \
         holds every connection."
    );
}

#[test]
fn a_background_checkout_waits_for_a_queued_interactive_one() {
    // The same property from the other side, and the one that actually
    // bounds the wait: a background checkout must not take a connection that
    // frees up while an interactive one is waiting for it. Taking it and then
    // yielding would be yielding after the fact, which is too late to help.
    let pool = pool_of_one();

    let blocking = pool.get().expect("the one connection");

    let (announced, arrived) = mpsc::channel();
    let waiting = std::thread::spawn({
        let pool = pool.clone();
        move || {
            announced.send(()).expect("the test is listening");
            let connection = pool.get_interactive().expect("a connection eventually");
            // Held, so the background attempt below has something to fail
            // against rather than a connection that is merely idle.
            std::thread::sleep(ENOUGH_TO_BLOCK);
            drop(connection);
        }
    });
    arrived.recv().expect("the interactive thread starts");
    while !pool.interactive_is_waiting() {
        std::hint::spin_loop();
    }

    drop(blocking);

    // With an interactive checkout queued, this must not be granted until
    // that one has come and gone.
    let started = std::time::Instant::now();
    let _connection = pool.get().expect("a connection eventually");
    let waited = started.elapsed();

    assert!(
        !pool.interactive_is_waiting(),
        "a background checkout was granted a connection with an interactive \
         one still queued behind it"
    );
    assert!(
        waited >= ENOUGH_TO_BLOCK / 2,
        "the background checkout was granted a connection immediately \
         ({waited:?}), so it did not wait for the interactive one that was \
         already queued"
    );
    waiting.join().expect("the interactive checkout finishes");
}
