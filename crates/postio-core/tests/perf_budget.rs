//! docs/PRODUCT.md §18 encodes three perf budgets as hard requirements, not
//! aspirations. This test is the CI-reliable half of enforcing them: it is a
//! deterministic check on a `Duration` value, not a wall-clock measurement,
//! so it runs in the ordinary `cargo test` job without the timing noise a
//! shared runner would introduce into a real benchmark.
//!
//! The wall-clock side lives in `benches/perf_budgets.rs`, which measures
//! placeholder workloads with criterion and calls the same [`check_budget`]
//! after each measurement.

use postio_core::perf_budget::{INTERACTION_BUDGET, SEARCH_BUDGET, STARTUP_BUDGET, check_budget};
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
