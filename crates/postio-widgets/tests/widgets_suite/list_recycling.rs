use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};

/// Far larger than the read-ahead window, so "bounded" and "one per item" cannot be the same
/// number.
const ITEMS: u32 = 5_000;

/// Measured at 205 on GTK 4.22.4; loose so GTK retuning its read-ahead does not fail, while one
/// widget per item misses by twelve times.
const WINDOW_CEILING: u32 = 400;

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
        measure("as the application builds it", |_| {}),
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

struct Setup {
    scroller: gtk::ScrolledWindow,
    view: gtk::ListView,
    store: gio::ListStore,
}

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

fn settle() {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    // POSTIO-FIXED-DEADLINE: spent in full, and nothing is waited *for* -- the
    // count this case asserts is reached synchronously inside `splice`, as the
    // comment above says. A dial would only lengthen a pump nobody reads.
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}
