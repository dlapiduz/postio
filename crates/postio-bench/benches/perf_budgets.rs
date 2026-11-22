#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use postio_core::perf_budget::{INTERACTION_BUDGET, SEARCH_BUDGET, STARTUP_BUDGET, check_budget};
use std::hint::black_box;
use std::time::Instant;

fn simulate_startup() -> u64 {
    (0..10_000u64).fold(0u64, |acc, i| acc.wrapping_add(black_box(i)))
}

fn simulate_message_list_scroll() -> u64 {
    (0..500u64).fold(0u64, |acc, i| acc.wrapping_add(black_box(i)))
}

fn simulate_search() -> u64 {
    (0..50_000u64).fold(0u64, |acc, i| acc.wrapping_add(black_box(i)))
}

fn bench_startup(c: &mut Criterion) {
    c.bench_function("startup", |b| b.iter(simulate_startup));

    let start = Instant::now();
    black_box(simulate_startup());
    check_budget(start.elapsed(), STARTUP_BUDGET).expect("startup placeholder exceeded budget");
}

fn bench_message_list_scroll(c: &mut Criterion) {
    c.bench_function("message_list_scroll", |b| {
        b.iter(simulate_message_list_scroll)
    });

    let start = Instant::now();
    black_box(simulate_message_list_scroll());
    check_budget(start.elapsed(), INTERACTION_BUDGET)
        .expect("message-list scroll placeholder exceeded budget");
}

fn bench_search(c: &mut Criterion) {
    c.bench_function("search", |b| b.iter(simulate_search));

    let start = Instant::now();
    black_box(simulate_search());
    check_budget(start.elapsed(), SEARCH_BUDGET).expect("search placeholder exceeded budget");
}

criterion_group!(
    benches,
    bench_startup,
    bench_message_list_scroll,
    bench_search
);
criterion_main!(benches);
