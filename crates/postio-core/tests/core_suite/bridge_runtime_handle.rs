use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_core::bridge::Bridge;
use postio_core::dispatch::Dispatcher;

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

    // `handle()` clones out of an `Option`; a runtime built per call would pass a naive test while
    // leaking one per caller.
    let first = bridge.handle();
    let second = bridge.handle();

    let on_first = first.block_on(async { std::thread::current().id() });
    let on_second = second.block_on(async { std::thread::current().id() });
    assert_eq!(
        on_first, on_second,
        "two handles drove two different threads, so they are not one runtime"
    );
}
