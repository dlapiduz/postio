//! The message row on a real display: its anatomy, its three densities, the
//! key hints only the focused row reveals, and the proof that it still reads
//! its colours off the cascade rather than out of the source.
//!
//! One test function, for the reason `gtk_style.rs` gives. Skips without a
//! display. Nothing here touches the network.

use chrono::{TimeZone, Utc};

use gtk::gdk;
use gtk::prelude::*;
use postio_config::Density;
use postio_gtk::list::Row;
use postio_gtk::list_view::MessageListView;
use postio_gtk::row::MessageRowView;
use postio_gtk::{fonts, style};
use postio_model::address::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// Canvas 1b's own first row.
fn canvas_row() -> Row {
    Row {
        id: MessageId::new(1),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Lena Tomlin"), "lena@example.com")),
        subject: Some("Re: maildir index rebuild is O(n²)".into()),
        preview: Some("Confirmed on 0.4.1 — the rebuild walks every…".into()),
        received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 14, 0).unwrap(),
        seen: false,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: true,
        thread_count: 14,
    }
}

#[test]
fn the_row_draws_the_canvas_anatomy_at_every_density() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    // Two rows, because "on the focused row only" is a claim about the row
    // next to it as much as about this one.
    let row = MessageRowView::new();
    row.set_row(Some(canvas_row()));
    let neighbour = MessageRowView::new();
    neighbour.set_row(Some(Row {
        id: MessageId::new(2),
        seen: true,
        ..canvas_row()
    }));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&row);
    column.append(&neighbour);
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&column));
    window.set_default_size(404, 300);
    window.present();
    pump();

    // ── the anatomy is announced, not merely drawn ───────────────────────
    let spoken = row.spoken();
    for expected in ["Unread", "Lena Tomlin", "maildir index", "14 in thread"] {
        assert!(
            spoken.contains(expected),
            "{spoken:?} never says {expected:?}"
        );
    }

    // ── three densities, three heights, tightest last ────────────────────
    let height = |density| {
        row.set_density(density);
        pump();
        row.measured_height(404)
    };
    let airy = height(Density::Airy);
    let comfortable = height(Density::Comfortable);
    let compact = height(Density::Compact);
    assert!(
        airy > comfortable && comfortable > compact,
        "airy {airy}, comfortable {comfortable}, compact {compact} do not tighten"
    );
    assert!(
        compact >= 30.0,
        "a {compact}px row is too tight to hold the anatomy"
    );

    // ── key hints belong to the focused row and to no other ──────────────
    row.set_density(Density::Airy);
    neighbour.set_density(Density::Airy);
    neighbour.grab_focus();
    pump();
    assert!(
        neighbour.shows_hints(),
        "the focused row reveals its key hints"
    );
    assert!(!row.shows_hints(), "the row beside it reveals nothing");
    let quiet = row.measured_height(404);

    row.grab_focus();
    pump();
    assert!(
        row.shows_hints(),
        "focus moved, and the hints moved with it"
    );
    assert!(!neighbour.shows_hints(), "the row it left went quiet again");
    assert!(
        row.measured_height(404) > quiet,
        "revealing the hints makes room for them"
    );

    // ── selected and focused are different states ────────────────────────
    // Focus is where the keyboard is; selection is what an action will hit.
    // A row that drew them the same way would make bulk actions feel
    // arbitrary, so the two have to be separately visible.
    row.set_selected(false);
    pump();
    let unselected = render(&window);
    row.set_selected(true);
    pump();
    assert!(row.is_selected());
    assert_ne!(
        unselected,
        render(&window),
        "a selected row draws no differently from an unselected one"
    );
    row.set_selected(false);
    pump();

    // ── an unbound row is a skeleton, not a crash and not a lie ──────────
    let waiting = MessageRowView::new();
    waiting.set_row(None);
    let holder = gtk::Window::new();
    style::track(&holder);
    holder.set_child(Some(&waiting));
    holder.set_default_size(404, 100);
    holder.present();
    pump();
    assert!(
        waiting.measured_height(404) > 0.0,
        "a placeholder still occupies its row"
    );
    assert!(
        !waiting.shows_hints(),
        "a row with nothing in it hints at nothing"
    );
    assert_eq!(waiting.spoken(), "", "a placeholder announces nothing");

    // ── the colours come from the cascade, not from the source ───────────
    // Nothing about the row changes here except the scheme, so two identical
    // renders would mean the widget had stopped listening to its tokens —
    // which for a hand-drawn widget is the one failure that looks fine.
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(adw::ColorScheme::ForceLight);
    pump();
    let light = render(&window);
    manager.set_color_scheme(adw::ColorScheme::ForceDark);
    pump();
    let dark = render(&window);
    assert_ne!(
        light, dark,
        "the row draws the same pixels in dark as in light"
    );
    manager.set_color_scheme(adw::ColorScheme::Default);

    // ── and the same row inside a real list ──────────────────────────────
    let pane = MessageListView::new();
    pane.set_mailbox("Inbox", 12);
    let list_window = gtk::Window::new();
    style::track(&list_window);
    list_window.set_child(Some(&pane));
    list_window.set_default_size(404, 600);
    list_window.present();
    pump();

    let seen = std::cell::Cell::new(0);
    pane.each_row(|_| seen.set(seen.get() + 1));
    assert_eq!(seen.get(), 0, "an empty model materialises no rows");
    assert_eq!(pane.model().n_items(), 0);
    assert_eq!(pane.density(), Density::Airy);

    // Rows arrive through the real page seam, and the list windows over
    // them: what it costs must not depend on how big the mailbox is.
    let materialise = |count: u32| {
        pane.model().set_source(std::rc::Rc::new(Sample {
            total: count,
            list: pane.model(),
        }));
        frames(&list_window, 6);
        assert_eq!(pane.model().n_items(), count);
        let widgets = std::cell::Cell::new(0usize);
        pane.each_row(|_| widgets.set(widgets.get() + 1));
        (widgets.get(), pane.model().resident_rows())
    };

    let (small_widgets, small_rows) = materialise(1_000);
    let (large_widgets, large_rows) = materialise(50_000);
    assert_eq!(
        small_widgets, large_widgets,
        "a 50,000-message mailbox materialises more widgets than a 1,000-message one"
    );
    let budget = postio_gtk::list::CACHE_PAGES * postio_gtk::list::PAGE_SIZE as usize;
    for resident in [small_rows, large_rows] {
        assert!(
            resident <= budget,
            "{resident} rows resident is past the {budget} budget"
        );
    }

    // Focusing the list puts the keyboard on a row, and exactly one row
    // reveals its hints.
    pane.grab_focus();
    frames(&list_window, 2);
    let hinting = std::cell::Cell::new(0);
    pane.each_row(|row| {
        if row.shows_hints() {
            hinting.set(hinting.get() + 1);
        }
    });
    assert_eq!(hinting.get(), 1, "the key hints belong to exactly one row");

    // A source that answers inside `request` used to take the process down
    // with it: the delivery emitted `items_changed` while the view was
    // part-way through its first layout, and GtkListView segfaulted. A fresh
    // pane, because that first layout is where it happened. The model holds
    // the delivery now, and this test proves it by continuing to exist.
    let hasty = MessageListView::new();
    let hasty_window = gtk::Window::new();
    style::track(&hasty_window);
    hasty_window.set_child(Some(&hasty));
    hasty_window.set_default_size(404, 600);
    hasty.model().set_source(std::rc::Rc::new(Impatient {
        total: 300,
        list: hasty.model(),
    }));
    hasty_window.present();
    frames(&hasty_window, 6);
    assert_eq!(hasty.model().n_items(), 300);
    let drawn = std::cell::Cell::new(0);
    hasty.each_row(|row| {
        if row.row().is_some() {
            drawn.set(drawn.get() + 1);
        }
    });
    assert!(
        drawn.get() > 0,
        "an impatient source's rows never reached a row widget"
    );
    hasty_window.destroy();

    // And the density switch re-measures what is already on screen rather
    // than rebuilding it.
    let before: Vec<f32> = heights(&pane);
    pane.set_density(Density::Compact);
    frames(&list_window, 2);
    assert_eq!(pane.density(), Density::Compact);
    let after = heights(&pane);
    assert!(
        after.iter().zip(&before).all(|(tight, airy)| tight < airy),
        "compact rows are not tighter than airy ones: {after:?} vs {before:?}"
    );
}

fn heights(pane: &MessageListView) -> Vec<f32> {
    let collected = std::cell::RefCell::new(Vec::new());
    pane.each_row(|row| collected.borrow_mut().push(row.measured_height(404)));
    collected.into_inner()
}

fn sample_row(id: i64) -> Row {
    Row {
        id: MessageId::new(id),
        seen: id % 2 == 0,
        ..canvas_row()
    }
}

/// A source that answers before `request` returns, which the contract
/// forbids and which used to be a segfault rather than a diagnostic.
struct Impatient {
    total: u32,
    list: postio_gtk::list::MessageList,
}

impl postio_gtk::list::PageSource for Impatient {
    fn total(&self) -> u32 {
        self.total
    }

    fn request(&self, page: u32) {
        let start = page * postio_gtk::list::PAGE_SIZE;
        let end = (start + postio_gtk::list::PAGE_SIZE).min(self.total);
        let rows = (start..end)
            .map(|index| sample_row(index as i64 + 1))
            .collect();
        self.list.deliver(page, rows);
    }
}

/// A page source that makes up a page on demand, the way `postio-91i` will
/// read one off the repository.
///
/// It holds a count rather than a vector on purpose: a source that kept the
/// whole mailbox in memory would be testing the opposite of what this file
/// claims. Delivery is never synchronous either, because `request` is called
/// from inside the model answering `item()`.
struct Sample {
    total: u32,
    list: postio_gtk::list::MessageList,
}

impl postio_gtk::list::PageSource for Sample {
    fn total(&self) -> u32 {
        self.total
    }

    fn request(&self, page: u32) {
        let start = page * postio_gtk::list::PAGE_SIZE;
        let end = (start + postio_gtk::list::PAGE_SIZE).min(self.total);
        let rows = (start..end)
            .map(|index| sample_row(index as i64 + 1))
            .collect();
        let list = self.list.clone();
        gtk::glib::idle_add_local_once(move || list.deliver(page, rows));
    }
}

/// Run the main loop until `window` has actually painted a frame.
///
/// `pump` is not a wait: a non-blocking iteration returns immediately when
/// nothing is pending, so it can spin through without the frame clock
/// ticking once. Anything that renders has to count frames instead.
fn frames(window: &gtk::Window, count: u32) {
    let left = std::rc::Rc::new(std::cell::Cell::new(count));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                gtk::glib::ControlFlow::Break
            } else {
                gtk::glib::ControlFlow::Continue
            }
        }
    });
    let context = gtk::glib::MainContext::default();
    let heartbeat = gtk::glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        gtk::glib::ControlFlow::Continue
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while left.get() > 0 && std::time::Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

fn render(window: &gtk::Window) -> Vec<u8> {
    frames(window, 3);
    let (width, height) = (window.width().max(1), window.height().max(1));
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);
    let node = snapshot.to_node().expect("the window drew something");
    let renderer = window
        .native()
        .and_then(|native| native.renderer())
        .expect("a realized window has a renderer");
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    renderer
        .render_texture(&node, Some(&bounds))
        .save_to_png_bytes()
        .to_vec()
}

fn pump() {
    for _ in 0..200 {
        gtk::glib::MainContext::default().iteration(false);
    }
}
