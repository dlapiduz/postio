//! What a scroll frame costs in Focus's list.
//!
//! docs/PRODUCT.md §18 gives an ordinary interaction 16ms, and scrolling the
//! list is the interaction that happens most. Focus's row
//! (`postio_gtk::list::RowWidget`) draws itself in one `snapshot()` to stay
//! inside that; this bench is what says whether it does.
//!
//! # What is measured, and what is not
//!
//! One iteration is **a screenful of rows rebound and drawn**: the work a
//! `GtkListView` does when you scroll a page. For each recycled row widget,
//! hand it a different conversation and build its render nodes -- the
//! subject and sender laid out and ellipsized against the column. That is
//! the part Postio wrote, and the part a regression would land in.
//!
//! Rasterising the render nodes is deliberately *not* in the loop. That
//! happens on the GPU, off this thread, and a criterion bench on a shared
//! runner cannot attribute it -- timing it would produce a number that moved
//! with the machine rather than with the code.
//!
//! # Running
//!
//! ```sh
//! cargo bench -p postio-bench --bench list_scroll
//! ```
//!
//! It needs a display, and skips without one. CI compiles benches but does
//! not time them: a shared runner is too noisy to trust for a millisecond
//! budget.

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor
// reaches bench targets. A bench is not public API.

use std::hint::black_box;
use std::time::Instant;

use chrono::{TimeZone, Utc};
use criterion::{Criterion, criterion_group, criterion_main};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::perf_budget::{INTERACTION_BUDGET, check_budget};
use postio_gtk::list::{FocusRow, RowObject, RowWidget};
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_model::listing::{MessageSummary, ThreadSummary};
use postio_widgets::list_model::ModelRow;

/// The list's width in a Focus window at its default size.
const WIDTH: i32 = 960;

/// Rows on screen in a 700px window at Focus's 40px row: the most a single
/// scroll frame has to rebind.
const SCREENFUL: i64 = 18;

/// A conversation with the length of text real mail has: a long subject
/// that has to be ellipsized is the expensive case, and the common one.
fn conversation(id: i64) -> FocusRow {
    let at = Utc.timestamp_opt(1_700_000_000 - id, 0).unwrap();
    let from = EmailAddress::new(Some("Ada Lovelace"), "ada@example.com");
    let subject = format!("[PATCH v{id} 2/7] sched: fix EEVDF lag accounting on the idle path");
    FocusRow::conversation(ThreadSummary {
        id: Some(ThreadId::new(id)),
        representative: MessageSummary {
            id: MessageId::new(id),
            thread: Some(ThreadId::new(id)),
            from: Some(from.clone()),
            subject: Some(subject.clone()),
            preview: Some(format!(
                "Peter, Vincent -- the lag decay was applied twice for run {id}, once in..."
            )),
            received_at: at,
            seen: id % 3 == 0,
            flagged: false,
            answered: false,
            send_state: None,
            send_at: None,
            has_attachments: id % 4 == 0,
            thread_count: (id % 9) as u32 + 1,
        },
        subject: Some(subject),
        participants: vec![from],
        message_count: (id % 9) as u32 + 1,
        unread_count: u32::from(id % 3 != 0),
        flagged: false,
        has_attachments: id % 4 == 0,
        last_at: at,
        marker: None,
        copies: Vec::new(),
    })
}

/// One recycled row widget in a window under Focus's stylesheet, so the
/// cascade resolves the way it does in the application.
fn mounted() -> Option<RowWidget> {
    if adw::init().is_err() {
        return None;
    }
    let display = gdk::Display::default()?;
    postio_gtk::style::install(&display);

    let row = RowWidget::default();
    let window = gtk::Window::new();
    window.set_child(Some(&row));
    window.set_default_size(WIDTH, 40);
    window.present();
    for _ in 0..200 {
        gtk::glib::MainContext::default().iteration(false);
    }
    Some(row)
}

/// Rebind and draw a screenful, the way scrolling a page does.
fn scroll_a_screenful(row: &RowWidget, objects: &[RowObject], from: i64) {
    let width = f64::from(row.width().max(WIDTH));
    let height = f64::from(row.height().max(40));
    for index in 0..SCREENFUL {
        let object = &objects[((from + index) as usize) % objects.len()];
        row.bind(object);
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(row)).snapshot(&snapshot, width, height);
        black_box(snapshot.to_node());
    }
}

fn bench_message_list_scroll(c: &mut Criterion) {
    let Some(row) = mounted() else {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    };
    // Many more rows than a screen, so a frame never rebinds what it drew.
    let objects: Vec<RowObject> = (1..=SCREENFUL * 20)
        .map(|id| {
            let object = RowObject::placeholder();
            object.set_item(conversation(id));
            object
        })
        .collect();

    let mut from = 0;
    c.bench_function("focus list scroll frame", |b| {
        b.iter(|| {
            from += SCREENFUL;
            scroll_a_screenful(&row, &objects, from);
        })
    });

    // Criterion reports; this fails. A bench that only reports is a bench
    // nobody notices regressing, which is why `postio-core`'s own budget
    // benches assert as well as measure.
    scroll_a_screenful(&row, &objects, 0); // warm the fonts and the palette
    let start = Instant::now();
    scroll_a_screenful(&row, &objects, SCREENFUL);
    let measured = start.elapsed();
    if let Err(exceeded) = check_budget(measured, INTERACTION_BUDGET) {
        panic!("a scroll frame is over budget: {exceeded:?}");
    }
}

criterion_group!(benches, bench_message_list_scroll);
criterion_main!(benches);
