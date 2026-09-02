//! The list header's sort control over a result set (#499).
//!
//! Over a mailbox the header says `Newest ▾` and the list really is in date
//! order. Over search results the list is *ranked* — bm25 folded with
//! recency and sender affinity — and the header used to go on saying
//! `Newest ▾` above it, which read as "the results are mixed up": the
//! ranking working as designed, labelled as a broken sort.
//!
//! The contract: over a result set the control says the order the results
//! are actually in, `Relevance ▾` by default; `o` (the same key that toggles
//! a thread's order) and a click both dispatch [`CommandId::ToggleResultOrder`]
//! through the registry; and leaving search puts the mailbox's own label
//! back.
//!
//! Skips without a display. One test function, for the reason `gtk_style.rs`
//! gives.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Context};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_search::ResultOrder;

pub fn the_sort_control_tells_the_truth_over_results() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();
    let list = window.list();

    // Every command the window delivers instead of handling itself, which is
    // where the app's search wiring listens for the toggle.
    let delivered: Rc<RefCell<Vec<CommandId>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_command({
        let delivered = delivered.clone();
        move |id| delivered.borrow_mut().push(id)
    });

    // -- a mailbox is in date order and says so ----------------------------

    assert_eq!(sort_text(&window), "Newest ▾");

    // -- a result set says the order it is actually in ---------------------

    list.set_result_order(Some(ResultOrder::Relevance));
    pump();
    assert_eq!(
        sort_text(&window),
        "Relevance ▾",
        "ranked results must not be labelled as a date sort"
    );

    list.set_result_order(Some(ResultOrder::Newest));
    pump();
    assert_eq!(sort_text(&window), "Newest ▾");

    // -- the key reaches the command, in the search context ----------------

    window.set_context(Context::Search);
    press(&window, "o");
    assert_eq!(
        delivered.borrow().as_slice(),
        [CommandId::ToggleResultOrder],
        "`o` over results means the same thing it means in a thread: toggle \
         the order of what I am looking at"
    );

    // -- and the control is clickable, dispatching the same command --------

    click_sort(&window);
    assert_eq!(
        delivered.borrow().as_slice(),
        [CommandId::ToggleResultOrder, CommandId::ToggleResultOrder],
        "the mouse reaches exactly the command the key reaches"
    );

    // -- leaving search restores the mailbox's own label -------------------

    list.set_result_order(None);
    pump();
    assert_eq!(sort_text(&window), "Newest ▾");

    // ...and with no result set on screen, the control is inert: there is
    // no other order a mailbox can be in, so a click must not dispatch.
    window.set_context(Context::List);
    click_sort(&window);
    assert_eq!(
        delivered.borrow().len(),
        2,
        "over a mailbox the control offers nothing to toggle"
    );

    window.destroy();
}

/// What the header's sort control is showing.
fn sort_text(window: &Window) -> String {
    let label = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-list-sort")
    })
    .expect("the header has a sort control");
    label
        .downcast::<gtk::Label>()
        .map(|label| label.text().to_string())
        .unwrap_or_default()
}

/// A pointer press-and-release on the sort control.
fn click_sort(window: &Window) {
    let label = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-list-sort")
    })
    .expect("the header has a sort control");
    // Synthesised through the gesture rather than a real event: GTK4 gives a
    // test no supported way to press a real pointer, and what is under test
    // is what a click *dispatches*, not GDK's delivery of it.
    let controllers = label.observe_controllers();
    let mut child = None;
    for index in 0..controllers.n_items() {
        if let Some(gesture) = controllers
            .item(index)
            .and_then(|object| object.downcast::<gtk::GestureClick>().ok())
        {
            child = Some(gesture);
            break;
        }
    }
    let gesture = child.expect("the sort control listens for clicks");
    gesture.emit_by_name::<()>("pressed", &[&1i32, &2.0f64, &2.0f64]);
    gesture.emit_by_name::<()>("released", &[&1i32, &2.0f64, &2.0f64]);
    pump();
}

/// One key press, resolved through the window's keymap.
fn press(window: &Window, key: &str) {
    let key = gtk::gdk::Key::from_name(key).expect("a named key");
    window.handle_key(key, gdk::ModifierType::empty());
    pump();
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    while context.iteration(false) {}
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
