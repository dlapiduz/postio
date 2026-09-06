//! Reading this process's CPU time, for cases that assert nothing was burned.
//!
//! # Why it is shared
//!
//! #1216 is answered by measurement rather than argument — each suspect is
//! excluded by running it and reading a clock — and every such case needs the
//! same two things: the clock, and proof the clock can see a burn at all. The
//! first of those was copied into one test file already, and a second copy is
//! how the two drift into disagreeing about what "no CPU" means.
//!
//! # The instrument, and why it is calibrated
//!
//! These cases are all *negative* assertions: "the engine used no CPU". The
//! cheapest way for one of those to pass is for the instrument to be broken —
//! a clock stuck at zero reports an idle engine however hard it spins. So
//! [`assert_the_clock_can_see_a_burn`] deliberately burns first and fails if
//! the reading does not move. That is not hypothetical caution: the first
//! version of #1216's SMTP case read zero because the engine never dialled,
//! and a capture of 6,000 good samples was once reported as zero because the
//! parser matched the wrong line.
//!
//! Linux only, which is v1's platform. There is no portable way to ask for
//! this and no second platform to be portable to yet.

use std::time::{Duration, Instant};

/// This process's CPU time so far, user plus system.
///
/// Fields 14 and 15 of `/proc/self/stat`, in clock ticks. The comm field can
/// contain spaces and parentheses, so the split is after the last `)` — the
/// standard way to parse this file, and the reason it is not a plain
/// `split_whitespace`.
///
/// It is the **process**, not the thread: a case using this belongs in a test
/// binary of its own, or a neighbour compiling a regex lands in the reading as
/// a spin.
pub fn cpu_time() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    let tail = &stat[stat.rfind(')').expect("the comm field ends") + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    // `tail` starts at the state field, which is field 3, so field 14 is
    // index 11 here and field 15 is index 12.
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    let ticks_per_second = 100; // `sysconf(_SC_CLK_TCK)`, 100 on every Linux this runs on.
    Duration::from_secs_f64((utime + stime) as f64 / ticks_per_second as f64)
}

/// Fails unless [`cpu_time`] moves when this process actually burns.
///
/// Call it before trusting a reading of zero. See the module docs for why a
/// negative assertion needs this and a positive one does not.
///
/// # Panics
///
/// If spinning does not move the clock — the reading cannot tell a spinning
/// engine from an idle one, so every assertion resting on it is vacuous.
pub fn assert_the_clock_can_see_a_burn() {
    /// Enough movement to be unambiguous, and small enough to be quick.
    const VISIBLE: Duration = Duration::from_millis(50);

    let before = cpu_time();
    // Spins until the *clock* moves, rather than for a fixed stretch of wall
    // time, so a machine that deschedules this thread lengthens the spin
    // instead of failing the assertion — and a machine that does not stops
    // early rather than burning a whole tenth of a second of somebody else's
    // core. The wall-clock bound is only the give-up.
    let give_up = Instant::now() + crate::scaled(Duration::from_secs(2));
    let mut counter = 0u64;
    let mut seen = Duration::ZERO;
    while seen < VISIBLE && Instant::now() < give_up {
        // A batch between readings: `/proc/self/stat` is a file read, and one
        // per turn would measure the reading rather than the spinning.
        for _ in 0..10_000 {
            counter = counter.wrapping_add(1);
        }
        seen = cpu_time().saturating_sub(before);
    }
    assert!(
        seen >= VISIBLE,
        "spinning read as {seen:?} of CPU after {counter} turns, so this \
         measurement cannot tell a spinning engine from an idle one"
    );
}
