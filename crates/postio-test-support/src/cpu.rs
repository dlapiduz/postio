//! Reading this process's CPU time, for cases that assert nothing was burned. The assertions are
//! negative, and the cheapest way for one to pass is a broken clock, so
//! [`assert_the_clock_can_see_a_burn`] burns first and fails if the reading does not move.
//! Linux only, which is v1's platform.

use std::time::{Duration, Instant};

/// Fields 14 and 15 of `/proc/self/stat`, split after the last `)` because the comm field can
/// contain spaces and parentheses.
/// The process, not the thread: a case using this belongs in a test binary of its own.
pub fn cpu_time() -> Duration {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("/proc/self/stat");
    let tail = &stat[stat.rfind(')').expect("the comm field ends") + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    let ticks_per_second = 100; // `sysconf(_SC_CLK_TCK)`, 100 on every Linux this runs on.
    Duration::from_secs_f64((utime + stime) as f64 / ticks_per_second as f64)
}

/// Call it before trusting a reading of zero: a clock stuck at zero reports an idle engine however
/// hard it spins.
///
/// # Panics
///
/// If spinning does not move the clock.
pub fn assert_the_clock_can_see_a_burn() {
    const VISIBLE: Duration = Duration::from_millis(50);

    let before = cpu_time();
    // Spins until the clock moves, not for a fixed wall time, so a descheduled thread lengthens the
    // spin instead of failing; the wall-clock bound is only the give-up.
    let give_up = Instant::now() + crate::scaled(Duration::from_secs(2));
    let mut counter = 0u64;
    let mut seen = Duration::ZERO;
    while seen < VISIBLE && Instant::now() < give_up {
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
