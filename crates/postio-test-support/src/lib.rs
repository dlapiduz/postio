//! One deadline, one backoff, one timeout message.
//!
//! # Why this crate exists
//!
//! The suite had **171 hand-rolled wait helpers** under ten names — `settle`
//! ×57, `settle_until` ×47, `pump` ×42, and seven more spellings — each file
//! defining its own deadline and its own backoff. They are all the same three
//! lines, and the duplication is not a tidiness complaint: it is why the
//! suite's flakiness could not be fixed.
//!
//! A wait long enough on an idle workstation is not long enough on a loaded
//! runner. With the deadline written into 171 places, the only available fix
//! is to enlarge the one copy that failed — which slows the suite for
//! everyone, permanently, and does nothing for the next runner that is
//! busier. Two landings on 2026-09-02 did exactly that: `postio-config`'s
//! watch debounce went 60 ms → 300 ms and its quiet period 400 → 600 ms, and
//! `app_suite`'s dwell went 80 ms → 500 ms. Neither made its test correct.
//!
//! With one definition, patience becomes a dial: see [`patience`].
//!
//! # What this is not
//!
//! **Not a speed-up.** Measured before writing it: the whole workspace spends
//! 108 s inside tests, and only about 4 s of that is fixed sleeping. Waits
//! that already poll a condition exit as soon as it holds and cost nothing.
//! The suite's time is spent linking 197 test binaries, which is #841. If
//! this crate ever appears in an argument about test *duration*, something
//! has been mismeasured.
//!
//! # No GTK, deliberately
//!
//! Most callers need to turn the GTK main loop while they wait, and this
//! crate still does not depend on GLib: [`settle_until`] takes the pump as a
//! closure. That is not indirection for its own sake —
//! `check-crate-boundaries.py` counts a **dev-dependency of a guarded crate**
//! against that crate's rules, so a support crate that pulled GLib would put
//! GTK inside `postio-core` and `postio-session`, both of which forbid it.
//! Passing the pump in keeps one primitive for every suite instead of one for
//! the GTK crates and another for everyone else.

use std::time::{Duration, Instant};

/// How long a wait may take before it is a failure, in milliseconds.
///
/// The default is deliberately generous. A wait that resolves in 3 ms returns
/// in 3 ms whatever this says — the deadline costs nothing until it is hit,
/// so the only thing a bigger number buys is tolerance, and the only thing a
/// smaller one buys is a flake.
const BASE_MILLIS: u64 = 5_000;

/// The environment variable that scales every deadline in the workspace.
pub const PATIENCE_VAR: &str = "POSTIO_TEST_PATIENCE";

/// How long to wait for something before calling it a failure.
///
/// `POSTIO_TEST_PATIENCE` multiplies it: `2` doubles every deadline in the
/// suite, `0.5` halves them. **This is the point of the crate.** Making CI
/// more patient than a workstation is a one-line change to a workflow, rather
/// than a pull request that enlarges a constant and slows every developer's
/// run forever.
///
/// An unparseable or non-positive value is ignored rather than honoured: a
/// typo in a workflow should not quietly set every deadline in the suite to
/// zero and turn every wait into an instant failure.
pub fn patience() -> Duration {
    patience_from(std::env::var(PATIENCE_VAR).ok().as_deref())
}

/// [`patience`] without the environment, so it can be tested without one.
///
/// Reading `POSTIO_TEST_PATIENCE` is a process-global act, and tests run in
/// parallel: a test that set the variable to check the scaling would change
/// the deadline of every wait running beside it. Splitting the parsing out
/// keeps the rule testable and leaves the environment alone.
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

/// How long to sleep between polls.
///
/// Small enough that a wait resolving quickly is not held up by the backoff,
/// large enough that polling does not spin a core.
const BACKOFF: Duration = Duration::from_millis(10);

/// Wait until `condition` holds, or fail with what was being waited for.
///
/// `label` is what the failure says. Phrase it as the thing that was supposed
/// to happen — "the body reaches the reader", not "wait_until failed" — since
/// it is the only description the next person gets of a test that timed out.
///
/// # Panics
///
/// If `condition` has not held within [`patience`]. Panicking rather than
/// returning a `bool` on purpose: every hand-rolled copy this replaces was
/// followed by an `assert!` that discarded the one useful fact, how long it
/// had actually waited.
pub fn wait_until(label: &str, condition: impl FnMut() -> bool) {
    settle_until(label, || {}, condition);
}

/// Wait until `condition` holds, turning a main loop between polls.
///
/// `pump` is called before each test of the condition. GTK callers pass
/// `|| while glib::MainContext::default().iteration(false) {}`; callers with
/// no loop to turn want [`wait_until`].
///
/// The condition is tested **after** pumping and once more after the deadline
/// passes, so a condition that becomes true during the final pump is not
/// reported as a timeout — which is its own flake, and one that only appears
/// on a loaded machine.
///
/// # Panics
///
/// If `condition` has not held within [`patience`].
pub fn settle_until(label: &str, pump: impl FnMut(), condition: impl FnMut() -> bool) {
    settle_until_within(patience(), label, pump, condition);
}

/// [`settle_until`] with an explicit deadline instead of [`patience`].
///
/// For the two cases where the shared deadline is the wrong one:
///
/// * proving something does **not** happen, which needs a duration because
///   absence cannot be polled for — wait a bounded time and assert the thing
///   never arrived;
/// * a test whose subject *is* a duration, like a debounce or a grace period.
///
/// Everything else wants [`settle_until`], so that raising `POSTIO_TEST_PATIENCE`
/// on a slow machine reaches it.
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
    loop {
        pump();
        if condition() {
            return;
        }
        if start.elapsed() >= limit {
            // One more turn and one more look: the deadline may have passed
            // while the loop was busy, and reporting a timeout for something
            // that is now true would be a lie.
            pump();
            if condition() {
                return;
            }
            panic!(
                "timed out after {:?} waiting for {label}\n\
                 (deadline is {PATIENCE_VAR}={} x {BASE_MILLIS}ms; \
                 raise it for a slow machine rather than editing this test)",
                start.elapsed(),
                std::env::var(PATIENCE_VAR).unwrap_or_else(|_| "1".into()),
            );
        }
        std::thread::sleep(BACKOFF);
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
    #[should_panic(expected = "timed out after")]
    fn a_condition_that_never_holds_says_what_it_waited_for() {
        // An explicit short deadline rather than the default: a crate about
        // not wasting the suite's time should not spend five seconds proving
        // its own timeout works, and shortening it via the environment would
        // change the deadline of every test running beside this one.
        settle_until_within(
            Duration::from_millis(20),
            "something that never happens",
            || {},
            || false,
        );
    }

    #[test]
    fn a_nonsense_patience_is_ignored_rather_than_honoured() {
        // A typo in a workflow must not set every deadline in the suite to
        // zero, which would turn every wait into an instant failure and read
        // as the whole suite breaking at once.
        for bad in [Some(""), Some("abc"), Some("0"), Some("-2"), Some("nonsense"), None] {
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
