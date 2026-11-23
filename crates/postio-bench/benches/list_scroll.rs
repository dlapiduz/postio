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

const WIDTH: i32 = 960;

const SCREENFUL: i64 = 18;

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
            to: Vec::new(),
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
        muted: None,
    })
}

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

    // Criterion reports; this fails, so a regression does not go unnoticed.
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
