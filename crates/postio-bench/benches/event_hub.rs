#![allow(missing_docs)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use postio_core::Event;
use postio_core::bridge::{EventHub, EventSink, EventStream};
use postio_model::{AccountId, MessageId};

const SUBSCRIBERS: [usize; 3] = [1, 2, 8];

const BATCH: u64 = 1_000;

fn small() -> Event {
    Event::ActionCompleted {
        description: "Archived".to_owned(),
        undoable: true,
    }
}

fn large() -> Event {
    Event::MessagesChanged {
        account: AccountId::new(1),
        messages: (1..=1000).map(MessageId::new).collect(),
    }
}

fn hub_with(n: usize) -> (EventHub, Vec<EventStream>) {
    let hub = EventHub::new();
    let streams = (0..n).map(|i| hub.subscribe(format!("sub-{i}"))).collect();
    (hub, streams)
}

fn drain(streams: &[EventStream]) {
    for stream in streams {
        while stream.try_next().is_some() {}
    }
}

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
