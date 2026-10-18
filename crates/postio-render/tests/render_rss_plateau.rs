//! Does opening message after message leave the process the size it was?
//! (#1503)
//!
//! Nothing else notices a leak in the reader. The counting instrument gates
//! documents, bytes and surfaces per operation; a path that allocates per
//! message and never frees passes every count and every suite. The reader is
//! `postio-render`, in process and shared by every frontend, so this cycles
//! the corpus through it -- compose, render, drop -- and reads the process's
//! resident set after each pass.
//!
//! It covers the shared render path only: composition (`postio_ui`'s
//! `reader::document`), Blitz, the display list, the low-res raster and the
//! text index. The GTK widget layer (tile cache, paintables) and the Swift
//! host are not exercised here and are not covered by this test.
//!
//! # The band
//!
//! Pass 1 is the warm-up: every fixture once, so the allocator has seen the
//! largest document and the font and style caches are full. Passes 2 and 3
//! are measured, and the assertion is on the **last half**: the resident
//! growth from the end of pass 2 to the end of pass 3 must stay under
//! `max(5% of the warm figure, 8 MiB)`. A relative band, so it holds on
//! any machine. Measured on macOS the real path grows 1 to 3.5 MiB over the
//! last half (the allocator wandering between identical passes) from a warm
//! 78 MiB, and the control grows 23 to 28 MiB, so 8 MiB is more than twice
//! the noise and a third of the leak.
//!
//! # The control
//!
//! A band nobody has seen exceeded proves nothing, so the same cycle runs
//! again retaining every rendered document, and must blow through the band.
//! That is the red run; the real path is not re-broken to get one. It runs
//! after the real cycle in the same test, because resident size is
//! per-process and the control's retained memory would pollute a neighbour.
//!
//! Resident size comes from `/proc/self/statm` on Linux and `ps` on macOS.
//!
//! POSTIO-MEASUREMENT: it renders the whole corpus six times over and reads a
//! process-wide figure that a neighbour in the same binary would disturb, so
//! it runs nightly (`--profile nightly`) rather than on every pull request:
//!
//! ```text
//! cargo nextest run -p postio-render --test render_rss_plateau --profile nightly --no-capture
//! ```

// The render suite's shared fonts and requests: this is a binary of its
// own, so the process's size is this test's alone.
#[path = "render_suite/support/mod.rs"]
mod support;

use postio_model::test_corpus;
use postio_render::RenderedDocument;

/// Resident set size of this process, in KiB.
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

/// Warm-up, then two measured passes: `(warm, mid, final)` in KiB.
fn cycle(mut keep: impl FnMut(RenderedDocument)) -> (u64, u64, u64) {
    let rendered = pass(&mut keep);
    assert!(rendered >= 40, "the corpus rendered only {rendered} bodies");
    let warm = rss_kib();
    pass(&mut keep);
    let mid = rss_kib();
    pass(&mut keep);
    (warm, mid, rss_kib())
}

/// The growth the last half may show: 5% of the warm figure, never less
/// than 8 MiB.
fn band(warm: u64) -> u64 {
    (warm / 20).max(8 * 1024)
}

#[test]
fn the_reader_path_plateaus() {
    let (warm, mid, last) = cycle(drop);
    let growth = last.saturating_sub(mid);
    println!(
        "reader path: warm {warm} KiB, mid {mid} KiB, final {last} KiB; \
         last-half growth {growth} KiB, band {} KiB",
        band(warm)
    );
    assert!(
        growth <= band(warm),
        "resident size kept growing across the last half of the cycle: \
         {mid} -> {last} KiB (band {} KiB)",
        band(warm)
    );

    // The control: retaining every document must be seen to leave the band.
    let mut kept = Vec::new();
    let (warm, mid, last) = cycle(|doc| kept.push(doc));
    let growth = last.saturating_sub(mid);
    println!(
        "control (retained {} documents): last-half growth {growth} KiB, band {} KiB",
        kept.len(),
        band(warm)
    );
    assert!(
        growth > band(warm),
        "the control retained {} documents and grew only {growth} KiB: the \
         band cannot see a leak",
        kept.len()
    );
}
