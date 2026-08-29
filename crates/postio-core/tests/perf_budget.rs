//! docs/PRODUCT.md §18 encodes three perf budgets as hard requirements, not
//! aspirations. This test is the CI-reliable half of enforcing them: it is a
//! deterministic check on a `Duration` value, not a wall-clock measurement,
//! so it runs in the ordinary `cargo test` job without the timing noise a
//! shared runner would introduce into a real benchmark.
//!
//! The wall-clock side lives in `benches/perf_budgets.rs`, which measures
//! placeholder workloads with criterion and calls the same [`check_budget`]
//! after each measurement.

use postio_core::perf_budget::{
    INTERACTION_BUDGET, SEARCH_BUDGET, STARTUP_BUDGET, SYNC_WRITE_BUDGET, check_budget,
};
use std::time::Duration;

#[test]
fn measurement_under_budget_passes() {
    assert!(check_budget(Duration::from_millis(1), STARTUP_BUDGET).is_ok());
    assert!(check_budget(Duration::from_micros(1), INTERACTION_BUDGET).is_ok());
    assert!(check_budget(Duration::from_millis(1), SEARCH_BUDGET).is_ok());
}

#[test]
fn measurement_exactly_at_budget_passes() {
    assert!(check_budget(STARTUP_BUDGET, STARTUP_BUDGET).is_ok());
    assert!(check_budget(INTERACTION_BUDGET, INTERACTION_BUDGET).is_ok());
    assert!(check_budget(SEARCH_BUDGET, SEARCH_BUDGET).is_ok());
}

#[test]
fn an_artificial_2x_regression_fails() {
    for budget in [STARTUP_BUDGET, INTERACTION_BUDGET, SEARCH_BUDGET] {
        let regressed = budget * 2;
        let err = check_budget(regressed, budget).unwrap_err();
        assert_eq!(err.measured, regressed);
        assert_eq!(err.budget, budget);
    }
}

#[test]
fn budgets_match_spec_md_section_18() {
    assert_eq!(STARTUP_BUDGET, Duration::from_millis(500));
    assert_eq!(INTERACTION_BUDGET, Duration::from_millis(16));
    assert_eq!(SEARCH_BUDGET, Duration::from_millis(100));
}

/// The sync write budget is not one of §18's three.
///
/// §18's budgets are about what a person waits for: startup, an interaction,
/// a search. This one is about what a first sync costs per message, which
/// nobody watches directly — they watch the mailbox fill. It is here because
/// #78 measured a first sync to be write-bound (a 1:12 fetch-to-write ratio
/// against a real account) with per-message cost growing as the store filled,
/// which makes it the number that decides when a first sync finishes.
///
/// Its value comes from measurement — `postio-runtime`'s `sync_writes` bench,
/// filed as #726 — not from a specification, so this test records what was
/// measured rather than what was promised.
#[test]
fn the_sync_write_budget_is_per_message() {
    // Roughly twice the 0.34-0.39 ms the bench measures, so a doubling trips
    // it and a slower disk does not. See the bench for the measured curve.
    assert_eq!(SYNC_WRITE_BUDGET, Duration::from_micros(750));
    assert!(
        SYNC_WRITE_BUDGET < INTERACTION_BUDGET,
        "one message must cost less than a whole interaction"
    );
}

#[test]
fn an_artificial_2x_sync_write_regression_fails() {
    let regressed = SYNC_WRITE_BUDGET * 2;
    let err = check_budget(regressed, SYNC_WRITE_BUDGET).unwrap_err();
    assert_eq!(err.measured, regressed);
    assert_eq!(err.budget, SYNC_WRITE_BUDGET);
}
