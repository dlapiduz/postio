//! Saved searches as a sidebar section, on a real display.
//!
//! Issue #10: a pinned `[filters]` entry is a query with a name, and the
//! sidebar's job is only to list it and say which one was picked -- running
//! it is `Window::run_search`'s job (`gtk_window_run_search.rs`), not this
//! widget's. Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::sidebar::{SavedSearch, SavedSearchAction, Sidebar};
use postio_gtk::{fonts, style};

pub fn saved_searches_list_keyboard_navigate_and_report_their_query() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let sidebar = Sidebar::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&sidebar));
    window.set_default_size(212, 700);
    window.present();
    pump();

    // ── nothing pinned yet: the section does not exist on screen ──────────
    assert!(rows(&sidebar).is_empty(), "no saved searches, no rows");
    assert!(
        !section_visible(&sidebar),
        "an account with no pinned filters should not show an empty heading"
    );

    // ── pinned filters appear, alphabetically ──────────────────────────────
    sidebar.set_saved_searches(&[
        SavedSearch {
            key: "attachments".to_string(),
            name: "attachments".to_string(),
            query: "has:attach".to_string(),
        },
        SavedSearch {
            key: "needs-reply".to_string(),
            name: "needs-reply".to_string(),
            query: "is:unread from:team".to_string(),
        },
    ]);
    pump();

    assert!(section_visible(&sidebar));
    assert_eq!(
        names(&sidebar),
        ["attachments".to_string(), "needs-reply".to_string()],
        "the widget draws them in exactly the order it was given -- #292's \
         reordering depends on that, so `set_saved_searches` must not \
         re-sort behind the caller's back"
    );

    // ── activating one reports its query, not its name ─────────────────────
    let picked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    sidebar.connect_search_selected({
        let picked = picked.clone();
        move |query| picked.borrow_mut().push(query)
    });

    let attachments_row = rows(&sidebar)[0].clone();
    let list: gtk::ListBox = attachments_row
        .parent()
        .and_then(|p| p.downcast().ok())
        .unwrap();
    list.select_row(Some(&attachments_row));
    pump();
    assert_eq!(*picked.borrow(), ["has:attach".to_string()]);

    // ── rows survive an update in place, like the folder rows do ──────────
    let before = rows(&sidebar);
    sidebar.set_saved_searches(&[
        SavedSearch {
            key: "attachments".to_string(),
            name: "attachments".to_string(),
            query: "has:attach".to_string(),
        },
        SavedSearch {
            key: "needs-reply".to_string(),
            name: "needs-reply".to_string(),
            query: "is:unread from:team".to_string(),
        },
    ]);
    pump();
    assert_eq!(rows(&sidebar), before);

    // ── clearing them takes the section back down ──────────────────────────
    sidebar.set_saved_searches(&[]);
    pump();
    assert!(rows(&sidebar).is_empty());
    assert!(!section_visible(&sidebar));

    window.destroy();
}

/// A window and sidebar with three saved searches (`a`, `b`, `c`), and every
/// row's own `gtk::ListBoxRow`. `None` means "skip": no display, or the
/// compositor never painted the frames the row geometry depends on.
///
/// Every context-menu case below builds its own window rather than sharing
/// one and opening several popovers across it in turn: `PopoverMenu` closes
/// itself on an idle/animation step rather than synchronously, so a second
/// `.popup()` before the first has actually finished tearing down raced it
/// -- observed directly as a `GtkListBox` warning stacking without end,
/// not assumed. One popover per window sidesteps the race instead of
/// chasing its timing.
fn three_searches() -> Option<(gtk::Window, Sidebar, Vec<gtk::ListBoxRow>)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let sidebar = Sidebar::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&sidebar));
    window.set_default_size(212, 700);
    window.present();
    pump();

    sidebar.set_saved_searches(&[
        SavedSearch {
            key: "a".to_string(),
            name: "a".to_string(),
            query: "subject:a".to_string(),
        },
        SavedSearch {
            key: "b".to_string(),
            name: "b".to_string(),
            query: "subject:b".to_string(),
        },
        SavedSearch {
            key: "c".to_string(),
            name: "c".to_string(),
            query: "subject:c".to_string(),
        },
    ]);
    pump();
    if !frames(&window, 2) {
        eprintln!("skipping: the compositor is not painting this window");
        return None;
    }

    let all_rows = rows(&sidebar);
    assert_eq!(all_rows.len(), 3);
    Some((window, sidebar, all_rows))
}

/// #292: the context menu's four verbs reach `connect_saved_search_action`
/// with the right key, on a row with somewhere to move in both directions.
///
/// Driven with [`Sidebar::test_open_saved_search_menu`] and
/// `WidgetExt::activate_action`, not a synthesized click -- see that
/// method's own doc for why. What's proved is the wiring between "this
/// action activated" and "the callback heard about it", not that a mouse
/// can reach the popover; the popover itself is stock `GtkPopoverMenu`
/// machinery this file has no reason to re-test.
pub fn the_context_menu_reaches_the_action_handler_with_the_right_key() {
    let Some((window, sidebar, all_rows)) = three_searches() else {
        return;
    };

    let heard: Rc<RefCell<Vec<(String, SavedSearchAction)>>> = Rc::new(RefCell::new(Vec::new()));
    sidebar.connect_saved_search_action({
        let heard = heard.clone();
        move |key, action| heard.borrow_mut().push((key, action))
    });

    sidebar.test_open_saved_search_menu(1.0, row_y(&all_rows[1]));
    assert!(
        sidebar.activate_action("savedsearch.rename", None).is_ok(),
        "rename should exist on a middle row"
    );
    assert!(sidebar.activate_action("savedsearch.move-up", None).is_ok());
    assert!(
        sidebar
            .activate_action("savedsearch.move-down", None)
            .is_ok()
    );
    assert!(sidebar.activate_action("savedsearch.delete", None).is_ok());
    assert_eq!(
        *heard.borrow(),
        vec![
            ("b".to_string(), SavedSearchAction::Rename),
            ("b".to_string(), SavedSearchAction::MoveUp),
            ("b".to_string(), SavedSearchAction::MoveDown),
            ("b".to_string(), SavedSearchAction::Delete),
        ],
        "all four should have fired for row b, in the order activated"
    );

    sidebar.test_close_saved_search_menu();
    window.destroy();
}

/// #292: the first row has nowhere to move up to, so that entry does not
/// exist -- not merely hidden, per [`Sidebar::open_saved_search_menu`]'s own
/// doc on why a menu-only gate would still leave it reachable.
pub fn the_first_row_has_no_move_up_entry() {
    let Some((window, sidebar, all_rows)) = three_searches() else {
        return;
    };

    sidebar.test_open_saved_search_menu(1.0, row_y(&all_rows[0]));
    assert!(
        sidebar
            .activate_action("savedsearch.move-up", None)
            .is_err(),
        "there is no menu entry to move the first row up further"
    );
    assert!(
        sidebar
            .activate_action("savedsearch.move-down", None)
            .is_ok(),
        "moving the first row down is still offered"
    );

    sidebar.test_close_saved_search_menu();
    window.destroy();
}

/// #292: the last row has nowhere to move down to.
pub fn the_last_row_has_no_move_down_entry() {
    let Some((window, sidebar, all_rows)) = three_searches() else {
        return;
    };

    sidebar.test_open_saved_search_menu(1.0, row_y(&all_rows[2]));
    assert!(
        sidebar.activate_action("savedsearch.move-up", None).is_ok(),
        "moving the last row up is still offered"
    );
    assert!(
        sidebar
            .activate_action("savedsearch.move-down", None)
            .is_err(),
        "there is no menu entry to move the last row down further"
    );

    sidebar.test_close_saved_search_menu();
    window.destroy();
}

/// Run the main loop until `window` has actually painted `count` frames.
///
/// `is_mapped()` becoming true is not enough -- observed directly while
/// writing this test: a row can be mapped with its bounds still the
/// pre-layout placeholder, because mapping and allocation are different
/// steps and only a real frame-clock tick runs the second one. This is
/// `gtk_focus_visible.rs::frames`, copied rather than shared: that file's
/// own module doc is the full story on why a non-blocking pump cannot
/// stand in for it, and this file has no dependency on that one.
fn frames(window: &gtk::Window, count: u32) -> bool {
    let left = std::rc::Rc::new(std::cell::Cell::new(count));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    });
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(10), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_secs(5);
    while left.get() > 0 && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
    left.get() == 0
}

/// A y-coordinate `row_at_y` will land on, in `row`'s own parent's
/// coordinate space -- `test_open_saved_search_menu`'s `(x, y)` is handed
/// straight to `GtkListBox::row_at_y` on that same list, so this has to
/// answer in the same space. `window` must have painted at least once
/// (`frames(&window, 1)`) before this is meaningful.
fn row_y(row: &gtk::ListBoxRow) -> f64 {
    let parent = row.parent().expect("a row in a list has a parent");
    let bounds = row
        .compute_bounds(&parent)
        .expect("a mapped row has bounds");
    (bounds.y() + bounds.height() / 2.0) as f64
}

/// Every saved-search row, in the order the sidebar draws them.
fn rows(sidebar: &Sidebar) -> Vec<gtk::ListBoxRow> {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-saved-search")
        .into_iter()
        .filter_map(|w| w.downcast().ok())
        .collect()
}

fn names(sidebar: &Sidebar) -> Vec<String> {
    rows(sidebar)
        .iter()
        .map(|row| {
            let label: gtk::Label = row.child().unwrap().downcast().unwrap();
            label.text().to_string()
        })
        .collect()
}

/// Whether the "Saved searches" heading and its list are on screen at all.
fn section_visible(sidebar: &Sidebar) -> bool {
    collect(
        sidebar.upcast_ref::<gtk::Widget>(),
        "postio-saved-searches-section",
    )
    .into_iter()
    .any(|w| w.is_visible())
}

/// Every widget in the tree carrying `class`, depth first.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}

fn pump() {
    for _ in 0..80 {
        glib::MainContext::default().iteration(false);
    }
}
