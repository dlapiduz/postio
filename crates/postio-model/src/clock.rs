//! The wall clock Postio reads, which a storyboard can stop.
//!
//! A row's "09:14" or "yesterday" is a function of the message and of *now*,
//! and so is the date a new draft or a queued send is stamped with. Filmed
//! twice, a storyboard would differ by whatever the clock did in between.
//! Code that stamps or draws a time asks [`now`] instead of `Local::now()`,
//! and a harness [`freeze`]s it so every frame is the same frame. It lives
//! here, at the bottom of the graph, so the model's stamps can read it too;
//! `postio_ui::clock` re-exports it. Unfrozen it is
//! the system clock. The freeze is process-wide, because the readers are
//! spread across widgets that share no handle.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use chrono::{DateTime, Local};

fn slot() -> &'static Mutex<Option<DateTime<Local>>> {
    static SLOT: OnceLock<Mutex<Option<DateTime<Local>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// The current instant: the frozen one if there is one, else the system's.
pub fn now() -> DateTime<Local> {
    slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(Local::now)
}

/// Stop the clock at `at`, for every thread, until [`thaw`].
pub fn freeze(at: DateTime<Local>) {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(at);
    *monotonic().lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

/// The monotonic clock, which an age on screen ("last sync 12s") is measured
/// on so it never jumps with the wall clock. Frozen with [`now`]: stopped at
/// the instant [`freeze`] was called, so an age reads the same in every frame.
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

/// The freeze is process-wide, so tests that use it, in this crate or
/// beside a type that reads it, take turns.
#[cfg(test)]
pub(crate) static TURN: Mutex<()> = Mutex::new(());

/// Let the clock run again.
pub fn thaw() {
    *slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
    *monotonic().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use super::TURN;

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
