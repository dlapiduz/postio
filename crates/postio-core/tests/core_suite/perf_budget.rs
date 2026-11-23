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

#[test]
fn the_sync_write_budget_is_per_message() {
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
