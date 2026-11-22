//! One deadline, one backoff, one timeout message. A wait long enough on an idle workstation is not
//! long enough on a loaded runner; with the deadline in one place, patience is a dial
//! ([`patience`]) instead of a constant enlarged in the file that flaked.
//! No GTK: [`settle_until`] takes the pump as a closure, because `check-crate-boundaries.py` counts
//! a dev-dependency of a guarded crate against that crate's rules, so a support crate pulling GLib
//! would put GTK inside `postio-core` and `postio-session`.

pub mod child;
pub mod cpu;
#[cfg(feature = "fidelity")]
pub mod fidelity;
pub mod listener;
#[cfg(feature = "logs")]
pub mod logs;

use std::time::{Duration, Instant};

/// Generous deliberately: a wait that resolves in 3 ms returns in 3 ms whatever this says, so a
/// bigger number buys only tolerance and a smaller one only a flake.
const BASE_MILLIS: u64 = 5_000;

pub const PATIENCE_VAR: &str = "POSTIO_TEST_PATIENCE";

/// `POSTIO_TEST_PATIENCE` multiplies it. An unparseable or non-positive value is ignored: a typo in
/// a workflow should not set every deadline to zero.
pub fn patience() -> Duration {
    patience_from(std::env::var(PATIENCE_VAR).ok().as_deref())
}

/// Without the environment: reading `POSTIO_TEST_PATIENCE` is process-global and tests run in
/// parallel, so a test setting it would change every wait beside it.
pub fn patience_from(raw: Option<&str>) -> Duration {
    let base = Duration::from_millis(BASE_MILLIS);
    match raw {
        Some(value) => match value.trim().parse::<f64>() {
            Ok(factor) if factor.is_finite() && factor > 0.0 => base.mul_f64(factor),
            _ => base,
        },
        None => base,
    }
}

/// For a suite whose deadline is deliberately not [`patience`]'s: it keeps its own base and still
/// answers to the dial. A nonsense multiplier is ignored.
pub fn scaled(base: Duration) -> Duration {
    let factor = std::env::var(PATIENCE_VAR)
        .ok()
        .and_then(|raw| raw.trim().parse::<f64>().ok())
        .filter(|f| f.is_finite() && *f > 0.0)
        .unwrap_or(1.0);
    base.mul_f64(factor)
}

/// Small enough that a quick wait is not held up, large enough that polling does not spin a core.
const BACKOFF: Duration = Duration::from_millis(10);

/// `label` is what the failure says: phrase it as the thing that was supposed to happen.
///
/// # Panics
///
/// If `condition` has not held within [`patience`]. A panic rather than a `bool`: every hand-rolled
/// copy discarded how long it had waited.
pub fn wait_until(label: &str, condition: impl FnMut() -> bool) {
    settle_until(label, || {}, condition);
}

/// `pump` is called before each test of the condition; GTK callers pass a main-loop turn. The
/// condition is tested after
/// pumping and once more after the deadline, so one that becomes true during the final pump is not
/// reported as a timeout.
///
/// # Panics
///
/// If `condition` has not held within [`patience`].
pub fn settle_until(label: &str, pump: impl FnMut(), condition: impl FnMut() -> bool) {
    settle_until_within(patience(), label, pump, condition);
}

/// For proving something does not happen (a bounded wait, since absence cannot be polled for) and
/// for a test whose subject is a duration;
/// everything else wants [`settle_until`], so `POSTIO_TEST_PATIENCE` reaches it.
///
/// # Panics
///
/// If `condition` has not held within `limit`.
pub fn settle_until_within(
    limit: Duration,
    label: &str,
    mut pump: impl FnMut(),
    mut condition: impl FnMut() -> bool,
) {
    let start = Instant::now();
    let mut longest_pump = Duration::ZERO;
    let mut turn = |pump: &mut dyn FnMut()| {
        let before = Instant::now();
        pump();
        longest_pump = longest_pump.max(before.elapsed());
    };

    loop {
        // Checked here as well as implied by the loop, so a `pump` that
        // overruns the deadline costs one overrun rather than one per
        // iteration (#1452).
        if start.elapsed() >= limit {
            // One more turn and one more look: the deadline may have passed
            // while the loop was busy, and reporting a timeout for something
            // that is now true would be a lie.
            turn(&mut pump);
            if condition() {
                return;
            }
            // A single pump that outlasts the whole budget is not a slow machine: it gets its own
            // sentence and does not offer the remedy that cannot work (#1452).
            if longest_pump >= limit {
                panic!(
                    "timed out after {:?} waiting for {label}, but the deadline \
                     was {limit:?}: a single pump took {longest_pump:?}, longer \
                     than the whole budget.\n\
                     (the budget was never the constraint, so {PATIENCE_VAR} \
                     will not help — something the pump waits on stopped \
                     answering)",
                    start.elapsed(),
                );
            }
            // The deadline passed in, not this crate's default: a message naming the default
            // described a budget never in force (#957).
            panic!(
                "timed out after {:?} waiting for {label}\n\
                 (the deadline was {limit:?}, which {PATIENCE_VAR}={} scales; \
                 raise it for a slow machine rather than editing this test)",
                start.elapsed(),
                std::env::var(PATIENCE_VAR).unwrap_or_else(|_| "1".into()),
            );
        }
        turn(&mut pump);
        if condition() {
            return;
        }
        std::thread::sleep(BACKOFF);
    }
}

/// The async form: building a runtime inside the closure panics under `#[tokio::test]`. `pump`
/// stays synchronous.
///
/// # Panics
///
/// As [`settle_until_within`].
pub async fn settle_until_async<P, C, Fut>(label: &str, mut pump: P, condition: C)
where
    P: FnMut(),
    C: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let limit = patience();
    let start = Instant::now();
    let mut longest_pump = Duration::ZERO;

    loop {
        if start.elapsed() >= limit {
            let before = Instant::now();
            pump();
            longest_pump = longest_pump.max(before.elapsed());
            if condition().await {
                return;
            }
            if longest_pump >= limit {
                panic!(
                    "timed out after {:?} waiting for {label}, but the deadline \
                     was {limit:?}: a single pump took {longest_pump:?}, longer \
                     than the whole budget.\n\
                     (the budget was never the constraint, so {PATIENCE_VAR} \
                     will not help — something the pump waits on stopped \
                     answering)",
                    start.elapsed(),
                );
            }
            panic!(
                "timed out after {:?} waiting for {label}\n\
                 (the deadline was {limit:?}, which {PATIENCE_VAR}={} scales; \
                 raise it for a slow machine rather than editing this test)",
                start.elapsed(),
                std::env::var(PATIENCE_VAR).unwrap_or_else(|_| "1".into()),
            );
        }
        let before = Instant::now();
        pump();
        longest_pump = longest_pump.max(before.elapsed());
        if condition().await {
            return;
        }
        tokio::time::sleep(BACKOFF).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_condition_already_true_returns_without_waiting() {
        let start = Instant::now();
        wait_until("something already done", || true);
        assert!(
            start.elapsed() < Duration::from_millis(BASE_MILLIS / 2),
            "a satisfied condition should not wait for the deadline"
        );
    }

    #[test]
    fn it_waits_for_a_condition_that_becomes_true() {
        let turns = Cell::new(0);
        settle_until(
            "the third turn",
            || turns.set(turns.get() + 1),
            || turns.get() >= 3,
        );
        assert_eq!(turns.get(), 3, "it should stop as soon as it is satisfied");
    }

    #[test]
    fn the_pump_runs_before_the_condition_is_first_tested() {
        // The ordering matters: a GTK caller's condition is only observable
        // *after* the main loop has been turned, so testing first would read
        // state from before anything was delivered and add a whole backoff
        // to every wait in the suite.
        let pumped = Cell::new(false);
        settle_until("a pumped condition", || pumped.set(true), || pumped.get());
    }

    #[test]
    #[should_panic(expected = "a single pump")]
    fn a_pump_that_blocks_past_the_deadline_says_so_instead_of_blaming_patience() {
        // #1452: a 5s deadline that fired at 120s because a pump blocked; the message must not
        // advise raising the dial.
        settle_until_within(
            Duration::from_millis(20),
            "something behind a pump that stopped answering",
            || std::thread::sleep(Duration::from_millis(300)),
            || false,
        );
    }

    #[test]
    #[should_panic(expected = "deadline was 20ms")]
    fn a_timeout_reports_the_deadline_it_actually_had() {
        // #957: a limit passed in has to speak for itself, not name this crate's default deadline.
        settle_until_within(
            Duration::from_millis(20),
            "something whose deadline is not the crate default",
            || {},
            || false,
        );
    }

    #[test]
    #[should_panic(expected = "raise it for a slow machine")]
    fn an_ordinary_timeout_still_points_at_the_dial() {
        settle_until_within(
            Duration::from_millis(20),
            "something that never happens under a cheap pump",
            || {},
            || false,
        );
    }

    #[test]
    fn a_blocking_pump_is_not_given_a_second_turn() {
        let pumps = Cell::new(0);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            settle_until_within(
                Duration::from_millis(20),
                "a condition behind a slow pump",
                || {
                    pumps.set(pumps.get() + 1);
                    std::thread::sleep(Duration::from_millis(120));
                },
                || false,
            );
        }));
        assert!(outcome.is_err(), "it should still time out");
        assert_eq!(
            pumps.get(),
            2,
            "one turn, then the deadline check, then the final turn -- never a third"
        );
    }

    #[test]
    #[should_panic(expected = "timed out after")]
    fn a_condition_that_never_holds_says_what_it_waited_for() {
        // An explicit short deadline: a crate about not wasting the suite's time should not spend
        // five seconds proving its timeout, and shortening it via the environment would change
        // every test beside this one.
        settle_until_within(
            Duration::from_millis(20),
            "something that never happens",
            || {},
            || false,
        );
    }

    #[test]
    fn a_nonsense_patience_is_ignored_rather_than_honoured() {
        for bad in [
            Some(""),
            Some("abc"),
            Some("0"),
            Some("-2"),
            Some("nonsense"),
            None,
        ] {
            assert_eq!(
                patience_from(bad),
                Duration::from_millis(BASE_MILLIS),
                "{bad:?} should fall back to the default deadline"
            );
        }
    }

    #[test]
    fn patience_scales_the_deadline() {
        assert_eq!(
            patience_from(Some("3")),
            Duration::from_millis(BASE_MILLIS * 3)
        );
        assert_eq!(
            patience_from(Some("0.5")),
            Duration::from_millis(BASE_MILLIS / 2)
        );
        assert_eq!(
            patience_from(Some("  2  ")),
            Duration::from_millis(BASE_MILLIS * 2),
            "a workflow that pads the value should still be honoured"
        );
    }
}
