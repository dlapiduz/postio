//! What it costs to emit an event through the hub, against the 16ms
//! interaction budget (#267, ADR 0013).
//!
//! ADR 0013 bet that "a read lock plus one `try_send` per subscriber is
//! invisible at mail-scale volume" and named its own falsifier: an emit-path
//! cost that shows up in the interaction budget with several subscribers
//! attached. This is the referee. It measures:
//!
//! * **One subscriber**, the only shape the application has today. The last
//!   subscriber is handed the envelope itself, so this pays a read lock and
//!   one `try_send` and no `Event` clone. That claim is what makes the hub
//!   free for the window, so it is the number to hold in place.
//! * **Two and eight subscribers**, where the per-subscriber `Event::clone`
//!   starts to count. The clone is not uniform across variants, so a small
//!   `ActionCompleted` and a `MessagesChanged` carrying 1000 ids are both
//!   swept.
//! * **Contention**: two producer threads emitting at once, the bus's
//!   handlers and the sync engine, to show a read lock does not serialise
//!   them.
//!
//! Subscriber queues are unbounded, so each sample emits a batch inside the
//! timed region and drains afterwards outside it; the reported time is per
//! emit.
//!
//! ```sh
//! cargo bench -p postio-bench --bench event_hub
//! ```

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor
// reaches bench targets. A bench is not public API.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use postio_core::Event;
use postio_core::bridge::{EventHub, EventSink, EventStream};
use postio_model::{AccountId, MessageId};

/// Subscriber counts: today's one, then where the clone starts to show.
const SUBSCRIBERS: [usize; 3] = [1, 2, 8];

/// Emits per timed batch, so the clock reads more than a few nanoseconds.
const BATCH: u64 = 1_000;

/// A small event, the common case.
fn small() -> Event {
    Event::ActionCompleted {
        description: "Archived".to_owned(),
        undoable: true,
    }
}

/// An event whose clone is not free: a thousand message ids.
fn large() -> Event {
    Event::MessagesChanged {
        account: AccountId::new(1),
        messages: (1..=1000).map(MessageId::new).collect(),
    }
}

/// A hub with `n` subscribers attached.
fn hub_with(n: usize) -> (EventHub, Vec<EventStream>) {
    let hub = EventHub::new();
    let streams = (0..n).map(|i| hub.subscribe(format!("sub-{i}"))).collect();
    (hub, streams)
}

/// Empty every subscriber's queue.
fn drain(streams: &[EventStream]) {
    for stream in streams {
        while stream.try_next().is_some() {}
    }
}

/// Time `iters` emits of `event` through `sink`; drain afterwards, untimed.
fn timed_emits(iters: u64, sink: &EventSink, event: &Event, streams: &[EventStream]) -> Duration {
    let mut total = Duration::ZERO;
    let mut left = iters;
    while left > 0 {
        let batch = left.min(BATCH);
        let start = Instant::now();
        for _ in 0..batch {
            black_box(sink.emit(event.clone()));
        }
        total += start.elapsed();
        drain(streams);
        left -= batch;
    }
    total
}

fn bench_emit(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_hub/emit");
    group.throughput(Throughput::Elements(1));
    for (name, event) in [("small", small()), ("1000_ids", large())] {
        for n in SUBSCRIBERS {
            let (hub, streams) = hub_with(n);
            let sink = hub.sink();
            group.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
                b.iter_custom(|iters| timed_emits(iters, &sink, &event, &streams));
            });
        }
    }
    group.finish();
}

fn bench_contended(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_hub/contended");
    group.throughput(Throughput::Elements(1));
    for n in SUBSCRIBERS {
        let (hub, streams) = hub_with(n);
        let event = small();
        group.bench_with_input(BenchmarkId::new("two_producers", n), &n, |b, _| {
            b.iter_custom(|iters| {
                let each = iters.div_ceil(2);
                let start = Instant::now();
                std::thread::scope(|scope| {
                    for _ in 0..2 {
                        let sink = hub.sink();
                        let event = &event;
                        scope.spawn(move || {
                            for _ in 0..each {
                                black_box(sink.emit(event.clone()));
                            }
                        });
                    }
                });
                let elapsed = start.elapsed();
                drain(&streams);
                elapsed
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_emit, bench_contended);
criterion_main!(benches);
