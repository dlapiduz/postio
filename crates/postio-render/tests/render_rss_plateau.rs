//! Does opening message after message leave the process the size it was?
//! (#1503)
//!
//! Nothing else notices a leak in the reader. The counting instrument gates
//! documents, bytes and surfaces per operation; a path that allocates per
//! message and never frees passes every count and every suite. The reader is
//! `postio-render`, in process and shared by every frontend, so this cycles
//! the corpus through it -- compose, render, drop -- and reads what the
//! process holds after each pass.
//!
//! It covers the shared render path only: composition (`postio_ui`'s
//! `reader::document`), Blitz, the display list, the low-res raster and the
//! text index. The GTK widget layer (tile cache, paintables) and the Swift
//! host are not exercised here and are not covered by this test.
//!
//! # What is measured
//!
//! The bytes the process has allocated and not freed, counted by this
//! binary's own `#[global_allocator]`. The render path has no C on it
//! (`check-renderer-is-memory-safe.py`) and maps no file per render (the
//! installed fonts are mapped once, when the `FontSet` is built), so every
//! byte it could leak passes through that allocator.
//!
//! It used to be the resident set, which also holds memory that a leak does
//! not move (#2189). It holds file-backed pages: the binary's own code and
//! the mapped fonts, 59 MiB of a 70 MiB process on a workstation, which the
//! kernel may drop and fault back in. It also holds freed memory that glibc
//! keeps in its per-thread arenas. Between identical passes it jumped by
//! 17 MiB: always on the nightly's runner, and in four runs of ten on a
//! workstation. On the workstation the jump was anonymous memory. Even after
//! `malloc_trim(0)` the anonymous part still moved by 6 MiB. Over the same
//! runs the live count stayed within 1 KiB. The resident set is still
//! printed, for a person to read.
//!
//! # The band
//!
//! Pass 1 is the warm-up: every fixture once, so the font and style caches
//! are full. Passes 2 and 3 are measured, and the assertion is on the **last
//! half**: the live bytes may grow by at most 1 MiB from the end of pass 2
//! to the end of pass 3. On Linux the real path grows by 0 KiB over that
//! half. The pass renders about a hundred bodies, so the band sees a leak of
//! about 10 KiB a message. The resident-set band it replaces was 8 MiB, so a
//! leak had to be about 80 KiB a message before it was seen.
//!
//! # The control
//!
//! A band nobody has seen exceeded proves nothing, so the same cycle runs
//! again retaining every rendered document, and must blow through the band.
//! That is the red run; the real path is not re-broken to get one. It runs
//! after the real cycle in the same test, because the count is per-process
//! and the control's retained memory would pollute a neighbour.
//!
//! POSTIO-MEASUREMENT: it renders the whole corpus six times over, so it
//! runs nightly (`--profile nightly`) rather than on every pull request:
//!
//! ```text
//! cargo nextest run -p postio-render --test render_rss_plateau --profile nightly --no-capture
//! ```

// The render suite's shared fonts and requests: this is a binary of its
// own, so the process's allocations are this test's alone.
#[path = "render_suite/support/mod.rs"]
mod support;

use postio_model::test_corpus;
use postio_render::RenderedDocument;

/// The allocator that counts live bytes. Implementing `GlobalAlloc` is
/// `unsafe impl` by definition. `postio-account`'s `imap_body_memory.rs` and
/// `postio-config`'s `validation_cost.rs` do the same, and
/// `check-lint-floor.py` records why this crate is `deny` rather than
/// `forbid`.
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

    /// Bytes allocated and not yet freed, in KiB.
    pub fn live_kib() -> u64 {
        (LIVE.load(Ordering::Relaxed) / 1024) as u64
    }
}

/// Resident set size of this process, in KiB: printed, not asserted.
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

/// One pass over the corpus: render every fixture with a body. Returns how
/// many rendered; each document goes to `keep` (or is dropped).
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

/// What the process holds at the end of a pass, in KiB.
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

/// Warm-up, then two measured passes: `(warm, mid, final)`.
fn cycle(mut keep: impl FnMut(RenderedDocument)) -> (Sample, Sample, Sample) {
    let rendered = pass(&mut keep);
    assert!(rendered >= 40, "the corpus rendered only {rendered} bodies");
    let warm = sample();
    pass(&mut keep);
    let mid = sample();
    pass(&mut keep);
    (warm, mid, sample())
}

/// The live growth the last half may show.
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

    // The control: retaining every document must be seen to leave the band.
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
