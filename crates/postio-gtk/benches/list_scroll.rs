//! What a scroll frame costs in the message list.
//!
//! docs/PRODUCT.md §18 gives an ordinary interaction 16ms, and scrolling the list is
//! the interaction that happens most. `postio_gtk::row` draws a row in one
//! `snapshot()` precisely to stay inside that; this bench is what says
//! whether it does.
//!
//! # What is measured, and what is not
//!
//! One iteration is **a screenful of rows rebound and laid out** — exactly
//! the work a `GtkListView` does when you scroll a page: for each recycled
//! row widget, hand it a different message, re-ellipsize the subject and the
//! snippet against the column width, and measure the height that comes out.
//! That is the part Postio wrote, and the part a regression would land in.
//!
//! Rasterising the resulting render nodes is deliberately *not* in the loop.
//! That happens on the GPU, off this thread, and a criterion bench on a
//! shared runner cannot attribute it — timing it would produce a number that
//! moved with the machine rather than with the code.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-gtk --bench list_scroll
//! ```
//!
//! It needs a display, and skips without one. CI compiles benches but does
//! not time them: a shared runner is too noisy to trust for a millisecond
//! budget.

use std::hint::black_box;
use std::time::Instant;

use chrono::{TimeZone, Utc};
use criterion::{Criterion, criterion_group, criterion_main};
use gtk::gdk;
use gtk::prelude::*;
use postio_config::Density;
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_gtk::list::Row;
use postio_gtk::row::MessageRowView;
use postio_gtk::{fonts, style};
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// The list's width in canvas 1b.
const WIDTH: i32 = 404;

/// Rows on screen at the airiest density in a 700px window — the most a
/// single scroll frame has to rebind.
const SCREENFUL: i64 = 16;

/// A row with the length of text real mail has: a long subject that has to
/// be ellipsized is the expensive case, and the common one.
fn message(id: i64) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(id)),
        from: Some(EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")),
        subject: Some(format!(
            "[PATCH v{id} 2/7] sched: fix EEVDF lag accounting on the idle path"
        )),
        preview: Some(format!(
            "Peter, Vincent — the lag decay was applied twice for run {id}, once in…"
        )),
        received_at: Utc.timestamp_opt(1_700_000_000 - id, 0).unwrap(),
        seen: id % 3 == 0,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: id % 4 == 0,
        thread_count: (id % 9) as u32 + 1,
    }
}

/// One recycled row widget in a window, so the cascade resolves the way it
/// does in the application.
fn mounted() -> Option<MessageRowView> {
    if adw::init().is_err() {
        return None;
    }
    let display = gdk::Display::default()?;
    // The fonts have to be installed before the first widget, or a
    // `PangoContext` caches the fallback family for the process — and this
    // bench would then be timing the wrong typeface.
    fonts::install().ok()?;
    style::install(&display);

    let row = MessageRowView::new();
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

/// Rebind and lay out a screenful, the way scrolling a page does.
fn scroll_a_screenful(row: &MessageRowView, from: i64) {
    for index in 0..SCREENFUL {
        row.set_row(Some(message(from + index)));
        black_box(row.measured_height(WIDTH));
    }
}

fn bench_message_list_scroll(c: &mut Criterion) {
    let Some(row) = mounted() else {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    };

    for (name, density) in [
        ("airy", Density::Airy),
        ("comfortable", Density::Comfortable),
        ("compact", Density::Compact),
    ] {
        row.set_density(density);
        let mut from = 0;
        c.bench_function(&format!("message-list scroll frame ({name})"), |b| {
            b.iter(|| {
                from += SCREENFUL;
                scroll_a_screenful(&row, from);
            })
        });
    }

    // Criterion reports; this fails. A bench that only reports is a bench
    // nobody notices regressing, which is why `postio-core`'s own budget
    // benches assert as well as measure.
    row.set_density(Density::Airy);
    scroll_a_screenful(&row, 0); // warm the fonts and the palette
    let start = Instant::now();
    scroll_a_screenful(&row, SCREENFUL);
    let measured = start.elapsed();
    if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
        panic!("a scroll frame is over budget: {exceeded:?}");
    }
}

criterion_group!(benches, bench_message_list_scroll);
criterion_main!(benches);
