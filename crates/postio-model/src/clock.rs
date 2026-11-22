//! The wall clock Postio reads, which a harness can `freeze` so storyboard frames are identical.
//! Code that stamps or draws a time asks `now`, not `Local::now()`. Process-wide, because readers
//! share no handle. Lives at the bottom of the graph so model stamps can read it;
//! `postio_ui::clock` re-exports it.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use chrono::{DateTime, Local};

fn slot() -> &'static Mutex<Option<DateTime<Local>>> {
    static SLOT: OnceLock<Mutex<Option<DateTime<Local>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

pub fn now() -> DateTime<Local> {
    slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(Local::now)
}

pub fn freeze(at: DateTime<Local>) {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(at);
    *monotonic().lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// Monotonic clock for on-screen ages, so they never jump with the wall clock; frozen with `now`.
pub fn instant() -> Instant {
    monotonic()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(Instant::now)
}

fn monotonic() -> &'static Mutex<Option<Instant>> {
    static SLOT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// The freeze is process-wide, so tests that use it take turns.
#[cfg(test)]
pub(crate) static TURN: Mutex<()> = Mutex::new(());

pub fn thaw() {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
    *monotonic().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::TURN;
    use super::*;
    use chrono::{Duration, TimeZone};

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

    #[test]
    fn the_monotonic_clock_stops_with_the_wall_clock() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        freeze(Local.with_ymd_and_hms(2026, 6, 2, 9, 0, 0).unwrap());
        let first = instant();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(instant(), first, "an age on screen would grow while frozen");
        thaw();
        let thawed = instant();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(instant() > thawed, "thawed, it runs");
    }
}
