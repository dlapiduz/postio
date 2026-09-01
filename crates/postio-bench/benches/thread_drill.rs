//! What it costs to fill a list view's read-ahead window.
//!
//! `postio-p44` started from a wrong diagnosis worth recording, because the
//! measurement that corrected it is what this bench exists to keep true.
//!
//! A 200-item model was producing 200 row widgets, which looked exactly like
//! `GtkListView` failing to recycle. It is not: GTK keeps a **read-ahead
//! window of about 205 rows** ready, and a 200-item model is simply smaller
//! than that window. A 1,000-item model builds the same 205, and so does a
//! 5,000-item one — `tests/gtk_list_recycling.rs` is the proof, and the
//! regression guard.
//!
//! So the cost that matters is not per *item*, it is per *row widget*, paid
//! once when a list is first populated. That is what this measures, and it is
//! why the thread row became a single `snapshot()` widget
//! (`crate::thread_row`) rather than four labels in a box:
//!
//! | Row shape | Filling the window |
//! |---|---|
//! | four labels in a `GtkBox` | 18.3 ms |
//! | one `snapshot()` widget | 6.8 ms |
//!
//! # What is measured, and what is not
//!
//! One iteration is **binding and measuring a read-ahead window's worth of
//! rows** — the work `GtkListView` does when a drill-in hands it a thread.
//! Rasterising is deliberately outside the loop, for the reason
//! `list_scroll.rs` gives: it happens on the GPU, off this thread, and a
//! bench cannot attribute it.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-gtk --bench thread_drill
//! ```
//!
//! It needs a display and skips without one. CI compiles benches but does not
//! time them: a shared runner is too noisy for a millisecond budget, and so
//! is a developer machine running four other builds — check `uptime` before
//! believing a number this prints.

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor now
// reaches bench targets -- the old per-crate `#![warn(missing_docs)]` in
// `lib.rs` never did. A bench is not public API, so documenting a
// macro-generated item would be ceremony rather than information.

use std::hint::black_box;
use std::time::Instant;

use chrono::{TimeZone, Utc};
use criterion::{Criterion, criterion_group, criterion_main};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_gtk::list::Row;
use postio_gtk::thread_row::ThreadRowView;
use postio_gtk::{fonts, style};
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// The thread column's width in canvas 3a.
const WIDTH: i32 = 404;

/// How many row widgets `GtkListView` keeps ready.
///
/// Measured, not guessed: `tests/gtk_list_recycling.rs` splices models of
/// 1,000 and 5,000 items and counts 205 factory calls for both.
const READ_AHEAD: i64 = 205;

/// How many times the budget check runs before taking the fastest.
const RUNS: i64 = 5;

/// The 1-minute load average, per core, above which this machine cannot
/// answer a millisecond question.
///
/// Half the cores free. Measured on this box: the same code sampled at
/// 11.2ms with a load of 1.7 and 17.6ms with a load of 5.9, and criterion's
/// own comparison called that +57% swing "no change detected" because the
/// variance had swallowed it. A budget asserted through that is not a budget,
/// it is a coin toss that fails the next person to run it.
const QUIET_ENOUGH: f64 = 0.5;

/// The 1-minute load average, if this system reports one.
fn load_average() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// A message with the length of text real mail has — a subject long enough to
/// need ellipsizing is the expensive case and the common one.
fn message(id: i64) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")),
        subject: Some(format!(
            "Re: [PATCH v{id} 2/7] sched: fix EEVDF lag accounting on the idle path"
        )),
        preview: None,
        received_at: Utc.timestamp_opt(1_700_000_000 - id, 0).unwrap(),
        seen: id % 5 != 0,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 200,
        participants: Vec::new(),
    }
}

/// One recycled row widget in a window, so the cascade resolves the way it
/// does in the application.
fn mounted() -> Option<ThreadRowView> {
    if adw::init().is_err() {
        return None;
    }
    let display = gdk::Display::default()?;
    // Fonts before the first widget, or a `PangoContext` caches the fallback
    // family for the process and this bench times the wrong typeface.
    fonts::install().ok()?;
    style::install(&display);

    let row = ThreadRowView::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&row));
    window.set_default_size(WIDTH, 200);
    window.present();
    for _ in 0..200 {
        gtk::glib::MainContext::default().iteration(false);
    }
    Some(row)
}

/// Bind and measure a read-ahead window's worth of rows, the way a drill-in
/// into a long thread does.
fn fill_the_window(row: &ThreadRowView, from: i64) {
    for index in 0..READ_AHEAD {
        row.set_row(Some(message(from + index)), (index + 1) as u32);
        black_box(row.measure(gtk::Orientation::Vertical, WIDTH));
    }
}

fn bench_thread_drill_in(c: &mut Criterion) {
    let Some(row) = mounted() else {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    };

    let mut from = 0;
    c.bench_function("thread drill-in, one read-ahead window", |b| {
        b.iter(|| {
            from += READ_AHEAD;
            fill_the_window(&row, from);
        })
    });

    // Criterion reports; this fails. A bench that only reports is a bench
    // nobody notices regressing, which is why `postio-core`'s own budget
    // benches assert as well as measure.
    //
    // Best of several runs rather than one. A single timing on a machine
    // that is also building something else measures the scheduler, not the
    // code — this bench's own first draft asserted a single shot and read
    // 18.8ms at load 5.9 for work criterion had just sampled at 11.2ms. The
    // fastest run is the one that was interrupted least, so it is the
    // closest thing to the cost of the code; noise can only ever make a run
    // slower.
    fill_the_window(&row, 0); // warm the fonts and the palette
    let best = (1..=RUNS)
        .map(|run| {
            let start = Instant::now();
            fill_the_window(&row, READ_AHEAD * run);
            start.elapsed()
        })
        .min()
        .expect("at least one run");

    // Report always; assert only when the machine can answer the question.
    // A busy box makes every measurement here a ceiling rather than a number,
    // and a budget gate that fires on someone else's build teaches people to
    // ignore it.
    let cores = std::thread::available_parallelism()
        .map(|cores| cores.get() as f64)
        .unwrap_or(1.0);
    let load = load_average();
    let quiet = load.is_none_or(|load| load < cores * QUIET_ENOUGH);

    eprintln!(
        "thread drill-in: best of {RUNS} was {best:?} against a {INTERACTION_BUDGET:?} \
         budget, at load {} on {cores} cores",
        load.map(|load| format!("{load:.2}"))
            .unwrap_or_else(|| "unknown".to_string()),
    );

    if !quiet {
        eprintln!(
            "thread drill-in: NOT asserting the budget — this machine is too \
             busy for the number above to mean anything. Re-run it quiet."
        );
        return;
    }

    if let Err(exceeded) = check_budget(best, INTERACTION_BUDGET) {
        panic!("filling a read-ahead window is over budget: {exceeded:?} (best of {RUNS})");
    }
}

criterion_group!(benches, bench_thread_drill_in);
criterion_main!(benches);
