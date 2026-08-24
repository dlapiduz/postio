//! Placeholder benches for the three docs/PRODUCT.md §18 budgets: startup,
//! message-list scroll and search. Each workload here is a stand-in until the
//! bead that owns the real thing lands and replaces it in place:
//!
//! - startup / message-list scroll: postio-91i (feed the message list from
//!   the repository over the bridge)
//! - search: postio-y47 (search performance benchmark under 100ms)
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-core --bench perf_budgets
//! ```
//!
//! CI compiles this bench (`cargo bench --workspace --no-run`) but does not
//! time it — a shared runner is too noisy to trust for millisecond budgets.
//! The regression gate that *does* run in CI is the deterministic
//! `tests/perf_budget.rs`, which checks `check_budget` directly against a
//! doubled duration rather than a live measurement.
//!
//! Each `bench_*` function below still asserts its own budget with a real
//! `Instant` measurement, in addition to feeding criterion — so running this
//! file locally fails loudly (panics) on a genuine regression, in a way a
//! `cargo bench --no-run` compile check never will.
//!
//! # Recording a new baseline
//!
//! ```sh
//! cargo bench -p postio-core --bench perf_budgets -- --save-baseline main
//! ```
//!
//! Criterion writes the baseline under `target/criterion/`, which is not
//! committed to the repository (it is machine-specific). Compare a later run
//! against it with `--baseline main`.

use criterion::{Criterion, criterion_group, criterion_main};
use postio_core::perf_budget::{INTERACTION_BUDGET, SEARCH_BUDGET, STARTUP_BUDGET, check_budget};
use std::hint::black_box;
use std::time::Instant;

/// Stands in for cold start work until postio-91i wires the real message
/// list up to the storage bridge.
fn simulate_startup() -> u64 {
    (0..10_000u64).fold(0u64, |acc, i| acc.wrapping_add(black_box(i)))
}

/// Stands in for windowed-scroll repaint work until postio-91i lands.
fn simulate_message_list_scroll() -> u64 {
    (0..500u64).fold(0u64, |acc, i| acc.wrapping_add(black_box(i)))
}

/// Stands in for an FTS5 query until postio-y47 lands the real benchmark.
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
