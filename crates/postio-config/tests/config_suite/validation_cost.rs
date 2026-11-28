//! Validation is held to an allocation count, not a stopwatch: a clock measures the machine (#917).
//! A single-test binary on purpose: a `#[global_allocator]` sees every allocation in the process.

#![allow(unsafe_code)]
// `unsafe impl GlobalAlloc` is the whole technique; no library code in this crate uses `unsafe`.

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

const NORMAL: &str = r#"[ui]
density = "compact"
theme = "dark"

[keys]
archive = "x"
summarize = "g s"

[saved_searches.needs-reply]
query = "is:unread from:team"
pinned = true

[sync]
idle = true
poll_interval_secs = 300
"#;

/// Measured at 246; the headroom is deliberate. The regressions this catches (re-parsing per key,
/// copying the table per entry) land in the thousands.
const NORMAL_CEILING: usize = 400;

const FEW: usize = 10;
const MANY: usize = 100;

/// `MANY` is ten times `FEW`: linear work is a ratio near ten, a quadratic pass would give a
/// hundred.
const GROWTH_CEILING: usize = 20;

fn with_filters(count: usize) -> String {
    let mut text = String::from("[ui]\ndensity = \"compact\"\n\n[keys]\narchive = \"x\"\n\n");
    for n in 0..count {
        text.push_str(&format!(
            "[saved_searches.saved-{n}]\nquery = \"is:unread from:team{n}\"\npinned = true\n\n"
        ));
    }
    text
}

/// Warmed first: the first validation pays for what the parser builds once.
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

    // It has to stay linear: a quadratic validation passes the ceiling on a small config and makes
    // the panel unusable on a large one.
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
