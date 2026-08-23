//! Perf budgets from spec.md §18, encoded as thresholds rather than hoped
//! for. `postio-core` hosts this because it is the crate every frontend and
//! subsystem eventually calls through, and because it is deliberately
//! UI-agnostic — the budgets apply to the assembled app, not to GTK.
//!
//! [`check_budget`] is the deterministic half of the gate: given a measured
//! [`Duration`], it says pass or fail with no dependence on the machine it
//! runs on. That determinism is what lets `an_artificial_2x_regression_fails`
//! (see `tests/perf_budget.rs`) run reliably in CI's `cargo test` job, unlike
//! a real wall-clock benchmark on a shared runner.
//!
//! The wall-clock half — actually measuring a workload — lives in
//! `benches/perf_budgets.rs` under criterion, with placeholder workloads
//! standing in for startup, message-list scroll and search until the crates
//! that own those (postio-91i, postio-y47) land and replace them in place.

use std::time::Duration;

/// Startup to usable UI with a populated database.
pub const STARTUP_BUDGET: Duration = Duration::from_millis(500);

/// An ordinary UI interaction, such as scrolling the message list.
pub const INTERACTION_BUDGET: Duration = Duration::from_millis(16);

/// A local search.
pub const SEARCH_BUDGET: Duration = Duration::from_millis(100);

/// A measurement exceeded its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetExceeded {
    /// What was actually measured.
    pub measured: Duration,
    /// The budget it was measured against.
    pub budget: Duration,
}

/// Checks a measured duration against a budget.
///
/// A measurement equal to the budget passes; only exceeding it fails.
pub fn check_budget(measured: Duration, budget: Duration) -> Result<(), BudgetExceeded> {
    if measured > budget {
        Err(BudgetExceeded { measured, budget })
    } else {
        Ok(())
    }
}
