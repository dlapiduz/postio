//! Config validation stays cheap, held to a budget a clock cannot tell you
//! (#917, and #100 before it).
//!
//! `config.toml` **is** the settings UI (canvas 3f): there is no OK button,
//! so the panel re-validates as the user types and the design advertises
//! "parsed in 2 ms". That is a budget rather than a boast, and the question
//! is only what enforces it.
//!
//! # Why this is not a stopwatch
//!
//! It used to be: `validating_a_normal_config_is_well_under_two_milliseconds`
//! averaged fifty runs and held each to 2 ms. On this repository's own
//! machine that assertion measures the *machine* — three sessions compile at
//! once routinely, and `.cargo/config.toml` pins `jobs = 2` precisely because
//! the box is oversubscribed. A test that fails when something else is busy
//! is worse than no test: it trains everyone to re-run it.
//!
//! #100 replaced the other perf budgets the same way and named the rule
//! CLAUDE.md now carries — *what gates a PR is the cause of each budget,
//! counted* — and converted every path it could reach from SQL. This one is
//! not a SQL path, so the countable cause is the work itself: validation is
//! CPU and allocation over a parsed table, and the allocator can be asked.
//!
//! # What it would catch
//!
//! The regressions that would actually break the promise: validation that
//! re-parses per key, that copies the whole table per entry, or that grows
//! super-linearly in the number of `[filters]`. Each shows up here as a
//! number, on every machine, identically.
//!
//! A single-test binary on purpose. A `#[global_allocator]` sees every
//! allocation in its process, and libtest runs a binary's tests in parallel
//! — so a second test in this file would be counted by this one.

#![allow(unsafe_code)]
// Implementing `GlobalAlloc` is `unsafe impl` by definition; that is the whole
// technique. `postio-account`'s `imap_body_memory.rs` does the same thing for
// the same kind of reason, and `check-lint-floor.py` records both. No library
// code in this crate uses `unsafe`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// An ordinary `config.toml`: what somebody actually has on disk.
const NORMAL: &str = r#"[ui]
density = "compact"
theme = "dark"

[keys]
archive = "x"
summarize = "g s"

[filters.needs-reply]
query = "is:unread from:team"
pinned = true

[sync]
idle = true
poll_interval_secs = 300
"#;

/// The ceiling for one validation of [`NORMAL`].
///
/// Measured at 246 on the pinned toolchain. The headroom is deliberate and
/// is not slack for a regression to hide in: an allocation count moves a
/// little with the standard library and with `toml`'s own internals, and the
/// regressions this exists to catch are not fifty allocations — they are
/// re-parsing per key, or copying the table per entry, which land in the
/// thousands. Tighten it when it is measured tighter, never to make a
/// number look good.
const NORMAL_CEILING: usize = 400;

/// How many `[filters]` the growth check compares.
const FEW: usize = 10;
const MANY: usize = 100;

/// `MANY` is ten times `FEW`, so linear work is a ratio near ten. Twice that
/// leaves room for the fixed cost every validation pays while still being a
/// long way below the hundred a quadratic pass would produce.
const GROWTH_CEILING: usize = 20;

fn with_filters(count: usize) -> String {
    let mut text = String::from("[ui]\ndensity = \"compact\"\n\n[keys]\narchive = \"x\"\n\n");
    for n in 0..count {
        text.push_str(&format!(
            "[filters.saved-{n}]\nquery = \"is:unread from:team{n}\"\npinned = true\n\n"
        ));
    }
    text
}

/// Allocations per validation of `text`, averaged over `runs`.
///
/// Warmed first: the first validation through a process pays for whatever
/// the parser builds once, and charging that to every run would measure
/// start-up rather than the work.
fn allocations_per_validation(text: &str, runs: usize) -> usize {
    for _ in 0..3 {
        let checked = postio_config::validate::check_str(text);
        assert!(
            checked.validation.is_valid(),
            "the fixture has to be valid, or this measures the error path: {}",
            checked.validation.status()
        );
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for _ in 0..runs {
        let _ = postio_config::validate::check_str(text);
    }
    (ALLOCATIONS.load(Ordering::Relaxed) - before) / runs
}

#[test]
fn validating_a_config_costs_a_bounded_amount_of_work() {
    let normal = allocations_per_validation(NORMAL, 20);
    assert!(
        normal <= NORMAL_CEILING,
        "validating an ordinary config allocates {normal} times, over the \
         {NORMAL_CEILING} this budgets. `config.toml` is the settings UI and \
         is re-validated as the user types, so this is the 2 ms the design \
         advertises, asserted as its cause rather than as a stopwatch reading"
    );

    // And it has to stay linear in the size of the file. A validation that
    // is quadratic in `[filters]` passes the ceiling above on a small config
    // and makes the panel unusable on a large one — which is exactly the
    // shape a wall-clock assertion on one fixture cannot see.
    let few = allocations_per_validation(&with_filters(FEW), 10);
    let many = allocations_per_validation(&with_filters(MANY), 10);
    let growth = many / few.max(1);
    assert!(
        growth <= GROWTH_CEILING,
        "{MANY} filters cost {many} allocations against {few} for {FEW} — a \
         factor of {growth}, where linear work is ten. Validation is doing \
         something per entry that looks at every other entry"
    );
}
