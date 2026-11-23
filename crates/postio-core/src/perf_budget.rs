//! Perf budgets from docs/PRODUCT.md §18 as thresholds. `check_budget` is the deterministic half
//! of the gate; the wall-clock half lives in `postio-bench`.

use std::time::Duration;

pub const STARTUP_BUDGET: Duration = Duration::from_millis(500);

pub const INTERACTION_BUDGET: Duration = Duration::from_millis(16);

pub const SEARCH_BUDGET: Duration = Duration::from_millis(100);

/// Per-message write cost during a first sync, which measured write-bound (#78, #726). About twice
/// the measured 0.34-0.39 ms, so a doubling trips it; not set from a contended live run.
pub const SYNC_WRITE_BUDGET: Duration = Duration::from_micros(750);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetExceeded {
    pub measured: Duration,
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
