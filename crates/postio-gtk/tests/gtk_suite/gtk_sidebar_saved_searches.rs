//! Saved searches as a sidebar section, on a real display.
//!
//! Issue #10: a pinned `[filters]` entry is a query with a name, and the
//! sidebar's job is only to list it and say which one was picked -- running
//! it is `Window::run_search`'s job (`gtk_window_run_search.rs`), not this
//! widget's. Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::sidebar::{SavedSearch, Sidebar};
use postio_gtk::{fonts, style};

pub fn saved_searches_list_keyboard_navigate_and_report_their_query() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
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
            name: "needs-reply".to_string(),
            query: "is:unread from:team".to_string(),
        },
        SavedSearch {
            name: "attachments".to_string(),
            query: "has:attach".to_string(),
        },
    ]);
    pump();

    assert!(section_visible(&sidebar));
    assert_eq!(
        names(&sidebar),
        ["attachments".to_string(), "needs-reply".to_string()],
        "sorted, the way the folder sections are"
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
            name: "needs-reply".to_string(),
            query: "is:unread from:team".to_string(),
        },
        SavedSearch {
            name: "attachments".to_string(),
            query: "has:attach".to_string(),
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
