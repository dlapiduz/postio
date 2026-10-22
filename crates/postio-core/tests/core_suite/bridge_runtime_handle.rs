//! Spawning work on the bridge's runtime from outside a handler.
//!
//! `Bridge::handle` and `Bridge::spawn` are how the sync engine gets its IDLE
//! loop and a body fetch onto the runtime: long-running work that does not
//! belong to any one command, so it cannot ride a handler. Everything else in
//! this crate's tests goes through the command queue, so until now the only
//! exercise these two got was `postio-runtime` using them — which means a
//! change here was checked by another crate's suite or not at all.
//!
//! That is also what makes them worth covering rather than merely counted:
//! `handle()` unwraps an `Option` on the grounds that "the runtime outlives
//! every method on Bridge", and the only thing standing behind that sentence
//! is `Drop` order. A test that spawns and waits is what says the sentence is
//! true while the bridge is alive.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;

/// A bus that answers nothing. These tests never send a command.
fn idle_bus() -> Dispatcher {
    Dispatcher::builder().build()
}

#[test]
fn work_spawned_on_the_bridge_runs_and_can_be_waited_for() {
    let (bridge, _events) = Bridge::new(idle_bus()).expect("the runtime starts");

    let ran = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&ran);
    let task = bridge.spawn(async move {
        counter.fetch_add(1, Ordering::SeqCst);
        "done"
    });

    // Joined through the same handle, because a caller that spawns background
    // work is the one that has to be able to wait for it at quit.
    let answer = bridge
        .handle()
        .block_on(async { task.await.expect("the task ran to completion") });

    assert_eq!(answer, "done");
    assert_eq!(
        ran.load(Ordering::SeqCst),
        1,
        "the future was never polled, so `spawn` handed it nowhere"
    );
}

#[test]
fn the_handle_is_the_same_runtime_every_time_it_is_asked_for() {
    let (bridge, _events) = Bridge::new(idle_bus()).expect("the runtime starts");

    // Not an identity check for its own sake: `handle()` clones out of an
    // `Option` every call, and a version that built a runtime per call would
    // pass a naive "does it work" test while leaking one per caller and
    // running the engine's loop somewhere nothing else could reach.
    let first = bridge.handle();
    let second = bridge.handle();

    let on_first = first.block_on(async { std::thread::current().id() });
    let on_second = second.block_on(async { std::thread::current().id() });
    assert_eq!(
        on_first, on_second,
        "two handles drove two different threads, so they are not one runtime"
    );
}
