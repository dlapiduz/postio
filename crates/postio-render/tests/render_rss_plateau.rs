//! Does opening message after message leave the process the size it was? Cycles the corpus through
//! compose, render, drop and asserts the live bytes counted by this binary's `#[global_allocator]`
//! grow by at most 1 MiB over the last pass. The resident set is printed, not asserted: it holds
//! file-backed pages and glibc arenas (#2189).
//! A control retains every document and must blow through the band; it runs after the real cycle
//! because the count is per-process.
//!
//! POSTIO-MEASUREMENT: it renders the whole corpus six times over, so it
//! runs nightly (`--profile nightly`) rather than on every pull request:

// The render suite's shared fonts and requests: this is a binary of its
// own, so the process's allocations are this test's alone.
#[path = "render_suite/support/mod.rs"]
mod support;

use postio_model::test_corpus;
use postio_render::RenderedDocument;

#[allow(unsafe_code)]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static LIVE: AtomicUsize = AtomicUsize::new(0);

    struct Counting;

    // SAFETY: every call goes to `System` unchanged; the counter is only
    // bookkeeping beside it.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) };
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let moved = unsafe { System.realloc(ptr, layout, new_size) };
            if !moved.is_null() {
                LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
                LIVE.fetch_add(new_size, Ordering::Relaxed);
            }
            moved
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    pub fn live_kib() -> u64 {
        (LIVE.load(Ordering::Relaxed) / 1024) as u64
    }
}

fn rss_kib() -> u64 {
    if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
        let pages: u64 = statm
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .expect("statm resident field");
        return pages * 4; // 4 KiB pages on every Linux this runs on
    }
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("ps prints the resident size in KiB")
}

fn pass(mut keep: impl FnMut(RenderedDocument)) -> usize {
    let mut rendered = 0;
    for name in test_corpus::names() {
        if let Some(request) = support::request(name, support::LIGHT) {
            keep(support::render(&request));
            rendered += 1;
        }
    }
    rendered
}

#[derive(Clone, Copy)]
struct Sample {
    live: u64,
    rss: u64,
}

fn sample() -> Sample {
    Sample {
        live: counting::live_kib(),
        rss: rss_kib(),
    }
}

fn cycle(mut keep: impl FnMut(RenderedDocument)) -> (Sample, Sample, Sample) {
    let rendered = pass(&mut keep);
    assert!(rendered >= 40, "the corpus rendered only {rendered} bodies");
    let warm = sample();
    pass(&mut keep);
    let mid = sample();
    pass(&mut keep);
    (warm, mid, sample())
}

const BAND_KIB: u64 = 1024;

#[test]
fn the_reader_path_plateaus() {
    let (warm, mid, last) = cycle(drop);
    let growth = last.live.saturating_sub(mid.live);
    println!(
        "reader path: live warm {} KiB, mid {} KiB, final {} KiB; last-half growth \
         {growth} KiB, band {BAND_KIB} KiB (resident {} / {} / {} KiB)",
        warm.live, mid.live, last.live, warm.rss, mid.rss, last.rss
    );
    assert!(
        growth <= BAND_KIB,
        "live bytes kept growing across the last half of the cycle: \
         {} -> {} KiB (band {BAND_KIB} KiB)",
        mid.live,
        last.live
    );

    let mut kept = Vec::new();
    let (_, mid, last) = cycle(|doc| kept.push(doc));
    let growth = last.live.saturating_sub(mid.live);
    println!(
        "control (retained {} documents): last-half growth {growth} KiB, band {BAND_KIB} KiB",
        kept.len()
    );
    assert!(
        growth > BAND_KIB,
        "the control retained {} documents and grew only {growth} KiB: the \
         band cannot see a leak",
        kept.len()
    );
}
