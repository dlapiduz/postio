//! Counters tests read, in `src/` because tests in several crates need them. A count gates a
//! performance claim, not a duration.

use std::sync::atomic::Ordering;

/// Resolution is quadratic in the number of commands; the classic message row paid it per row built
/// (#1216).
pub fn keymap_resolutions() -> u64 {
    crate::config::RESOLUTIONS.load(Ordering::Relaxed)
}
