//! `GtkListView` keeps a bounded window of rows, whatever the model holds.
//!
//! # The measurement that corrected postio-p44
//!
//! The bead was filed saying recycling was defeated: a 200-item model built
//! 200 row widgets, in both of Postio's list views. That is true, and it is
//! not what it looked like.
//!
//! `GtkListView` keeps a **read-ahead window** of roughly 205 rows ready —
//! considerably more than a screenful, and fixed. A 200-item model is smaller
//! than that window, so "one widget per item" and "the whole read-ahead
//! window" were the same number, and the test size made a healthy list view
//! look broken. Scale the model past the window and the truth shows:
//!
//! | Model | Row widgets built |
//! |---|---|
//! | 50 | 50 |
//! | 200 | 200 |
//! | 1,000 | 205 |
//! | 5,000 | 205 |
//!
//! So a 60,000-message mailbox costs 205 row widgets, not 60,000, and
//! `docs/PRODUCT.md` §18's "never materialise a mailbox" was never being violated.
//!
//! What that leaves is a real cost — filling those rows, once, when a list is
//! first populated — paid per *widget* rather than per item. That is why the
//! thread row became a single `snapshot()` widget: see
//! `benches/thread_drill.rs`.
//!
//! # What this guards
//!
//! That the window stays bounded. If a future change makes a list view build
//! one widget per model item for real, a large mailbox would materialise and
//! the application would stop being usable at scale — so this splices a model
//! far larger than the window and insists the count does not follow it.
//!
//! The per-case table is kept deliberately: each case was a suspect while
//! this was still being read as a recycling failure, and a regression should
//! say *which* knob broke it rather than only that something did.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

/// A model far larger than the read-ahead window, so "bounded" and "one per
/// item" cannot come out as the same number.
const ITEMS: u32 = 5_000;

/// The most row widgets a healthy `GtkListView` builds for [`ITEMS`].
///
/// Measured at 205 on GTK 4.22.4. The ceiling is loose so a future GTK
/// retuning its read-ahead does not fail this, while one widget per item —
/// the failure actually worth catching — misses it by a factor of twelve.
const WINDOW_CEILING: u32 = 400;

/// One configuration, and how many rows it built.
struct Case {
    name: &'static str,
    made: u32,
}

pub fn a_list_view_builds_a_bounded_window_however_big_the_model_is() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let cases = vec![
        // The shape `crate::list_view` and `crate::thread` both build.
        measure("as the application builds it", |_| {}),
        // Each of these was a suspect while postio-p44 was still being read
        // as a recycling failure. None of them changes the count, which is
        // most of how we know the configuration was never the problem.
        measure("horizontal policy Automatic", |setup| {
            setup
                .scroller
                .set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        }),
        measure("propagate natural height", |setup| {
            setup.scroller.set_propagate_natural_height(true);
        }),
        measure("no SingleSelection in the middle", |setup| {
            setup
                .view
                .set_model(Some(&gtk::NoSelection::new(Some(setup.store.clone()))));
        }),
    ];

    for case in &cases {
        eprintln!("p44: {:<34} {} row widgets", case.name, case.made);
    }

    for case in &cases {
        assert!(
            case.made <= WINDOW_CEILING,
            "`{}` built {} row widgets for a {ITEMS}-item model. `GtkListView` \
             is meant to keep a bounded read-ahead window whatever the model \
             holds; one widget per item would mean a real mailbox \
             materialises. All cases: {}",
            case.name,
            case.made,
            cases
                .iter()
                .map(|case| format!("{} = {}", case.name, case.made))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

/// The widgets one case is built from, so a case can vary one of them.
struct Setup {
    scroller: gtk::ScrolledWindow,
    view: gtk::ListView,
    store: gio::ListStore,
}

/// Builds the application's list shape, applies `vary`, fills the model, and
/// answers how many row widgets the factory was asked to make.
fn measure(name: &'static str, vary: impl FnOnce(&Setup)) -> Case {
    let made = Rc::new(Cell::new(0u32));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup({
        let made = Rc::clone(&made);
        move |_, item| {
            made.set(made.get() + 1);
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                item.set_child(Some(&gtk::Label::new(Some("placeholder"))));
            }
        }
    });

    let store = gio::ListStore::new::<gtk::StringObject>();
    let cursor = gtk::SingleSelection::new(Some(store.clone()));
    cursor.set_autoselect(false);
    let view = gtk::ListView::new(Some(cursor), Some(factory));
    view.set_vexpand(true);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_child(Some(&view));

    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&scroller);

    let window = gtk::Window::new();
    window.set_default_size(404, 700);
    window.set_child(Some(&column));

    let setup = Setup {
        scroller,
        view,
        store: store.clone(),
    };
    vary(&setup);

    window.present();
    settle();

    // Filled after the window is up, which is when the application fills it:
    // a page arrives, or a drill-in happens, long after the viewport exists.
    // The count comes out the same either way — that was measured too, along
    // with filling it a page at a time.
    let items: Vec<gtk::StringObject> = (0..ITEMS)
        .map(|index| gtk::StringObject::new(&format!("row {index}")))
        .collect();
    store.splice(0, 0, &items);
    settle();

    window.destroy();
    settle();
    Case {
        name,
        made: made.get(),
    }
}

/// Run the main loop long enough for GTK to lay out and realise what it means
/// to.
///
/// The count does not actually depend on this: it is reached synchronously
/// inside `splice`, which is worth knowing, because it means the cost lands
/// on the frame that populates the list rather than on some later idle.
fn settle() {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}
