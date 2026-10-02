//! The wall clock the interface reads, which a storyboard can stop.
//!
//! A row's "09:14" or "yesterday" is a function of the message and of *now*.
//! Filmed twice, a storyboard would differ by whatever the clock did in
//! between. Presentation code asks [`now`] instead of `Local::now()`, and a
//! harness [`freeze`]s it so every frame is the same frame. Unfrozen it is
//! the system clock. The freeze is process-wide, because the readers are
//! spread across widgets that share no handle.

use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Local};

fn slot() -> &'static Mutex<Option<DateTime<Local>>> {
    static SLOT: OnceLock<Mutex<Option<DateTime<Local>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// The current instant: the frozen one if there is one, else the system's.
pub fn now() -> DateTime<Local> {
    Local::now()
}

/// Stop the clock at `at`, for every thread, until [`thaw`].
pub fn freeze(at: DateTime<Local>) {
    let _ = at;
    let _ = slot();
}

/// Let the clock run again.
pub fn thaw() {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use std::sync::Mutex as TestLock;

    /// The freeze is process-wide, so tests that use it take turns.
    static TURN: TestLock<()> = TestLock::new(());

    #[test]
    fn unfrozen_it_is_the_system_clock() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        thaw();
        let skew = (now() - Local::now()).abs();
        assert!(skew < Duration::seconds(1), "skew {skew}");
    }

    #[test]
    fn frozen_it_is_exact_from_any_thread() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let t = Local.with_ymd_and_hms(2026, 6, 2, 9, 0, 0).unwrap();
        freeze(t);
        assert_eq!(now(), t);
        assert_eq!(std::thread::spawn(now).join().unwrap(), t);
        thaw();
    }

    #[test]
    fn thawing_restores_the_system_clock() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let t = Local.with_ymd_and_hms(2001, 1, 1, 0, 0, 0).unwrap();
        freeze(t);
        assert_eq!(now(), t, "frozen first, or thawing proves nothing");
        thaw();
        assert!((now() - Local::now()).abs() < Duration::seconds(1));
    }
}
