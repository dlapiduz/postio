//! Canvas 2b's left column on a real display: scope, refine, and the
//! keyboard.
//!
//! The rule about *which* refinements are worth offering is pure, and lives
//! in `postio-search`. What needs a display is what the column does with
//! them: that it takes the sidebar's place while a search is on screen and
//! gives it back afterwards, that switching scope asks again rather than
//! editing the query, that a chip appends a token Backspace can pop, and that
//! all of it is reachable from the keyboard.
//!
//! Skips without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use crate::settle as pump;
use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::search::View;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};

pub fn the_scope_column_narrows_a_search_without_retyping_it() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let view = View::attach(&window.shell(), &window.finder());
    window.present();
    pump();

    let finder = window.finder();
    let panel = view.panel();

    // Every question the box asks, and the scope it was asked under — which
    // is what whoever owns the store would read to build its request.
    let asked: Rc<RefCell<Vec<(String, Scope)>>> = Rc::new(RefCell::new(Vec::new()));
    let live = finder.live().expect("the box has a readout");
    live.connect_run({
        let asked = asked.clone();
        let view = view.clone();
        move |query, _| {
            asked
                .borrow_mut()
                .push((query.input().to_owned(), view.scope()))
        }
    });

    // -- at rest the sidebar is the sidebar -------------------------------

    assert!(!view.is_active());
    assert!(sidebar_shows_folders(&window), "no search, so no column");

    // -- typing hands the sidebar over ------------------------------------

    window.open_finder(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: "maildir".to_owned(),
    });
    pump();
    assert!(view.is_active());
    assert!(
        !sidebar_shows_folders(&window),
        "the column takes the sidebar's place rather than stacking under it"
    );

    // -- the counts are what was measured, per scope ----------------------

    view.set_facets(&facets(), 14);
    pump();
    assert_eq!(
        scope_rows(&window),
        [
            ("All mail".to_string(), "14".to_string()),
            ("Inbox only".to_string(), "6".to_string()),
            ("Lists".to_string(), "8".to_string()),
        ],
        "every scope says what is behind it, so switching is never a guess"
    );

    // -- only the refinements worth offering are drawn --------------------

    assert_eq!(
        panel.offered(),
        ["is:unread", "in:lkml", "larger:1M", "is:flagged"],
        "gentlest narrowing first; the one that keeps nothing is not offered"
    );

    // -- switching scope asks again, and does not touch the query ---------

    let before = asked.borrow().len();
    // `set_scope` is the application putting the panel somewhere — restoring
    // a session, say — not the user choosing. It must not fire a search.
    panel.set_scope(Scope::Lists);
    pump();
    assert_eq!(asked.borrow().len(), before, "nobody chose that");
    assert_eq!(panel.scope(), Scope::Lists);

    pick_scope(&window, 1);
    pump();
    assert_eq!(
        asked.borrow().last().expect("switching scope asks again"),
        &("maildir".to_string(), Scope::Inbox),
        "the same query, against the new scope"
    );
    assert_eq!(
        finder.query().text,
        "maildir",
        "the box still says what was typed; the scope is not a token"
    );

    // -- a refine chip appends a token Backspace can pop ------------------

    click_chip(&window, "is:unread");
    pump();
    assert_eq!(finder.query().text, "maildir is:unread");
    assert_eq!(
        finder.chips().len(),
        1,
        "it landed as an ordinary chip, not as a filter held somewhere else"
    );

    // Clicking it again must not double it — the chip may still be on screen
    // when a second click lands, before anything has answered.
    click_chip(&window, "is:unread");
    pump();
    assert_eq!(finder.query().text, "maildir is:unread");

    // -- nothing on offer says why, rather than going blank ---------------

    view.set_facets(&Facets::default(), 0);
    pump();
    assert!(panel.offered().is_empty());
    assert!(
        refine_note(&window).contains("Nothing matched"),
        "an empty column has to say which kind of empty it is: {:?}",
        refine_note(&window)
    );

    view.set_facets(
        &Facets {
            scopes: Vec::new(),
            // Everything left is unread, so narrowing by it would do nothing
            // visible.
            refinements: vec![refinement("is:unread", 14)],
        },
        14,
    );
    pump();
    assert!(panel.offered().is_empty());
    assert!(
        refine_note(&window).contains("alike"),
        "a different empty, and a different reason: {:?}",
        refine_note(&window)
    );

    // -- Tab moves into refine, per the canvas footer ---------------------

    view.set_facets(&facets(), 14);
    pump();
    assert!(
        finder.press_tab(),
        "there are chips, so Tab has somewhere to go"
    );
    pump();
    assert!(
        focused_is_a_chip(&window),
        "the keyboard is on the first refine chip"
    );

    view.set_facets(&Facets::default(), 0);
    pump();
    assert!(
        !finder.press_tab(),
        "nothing to refine, so Tab stays ordinary rather than silently doing nothing"
    );

    // -- leaving search puts the sidebar back exactly as it was -----------

    window.close_finder();
    pump();
    assert!(!view.is_active());
    assert!(
        sidebar_shows_folders(&window),
        "Esc has to cost nothing; the folder list never went away"
    );

    window.destroy();
}

fn facets() -> Facets {
    Facets {
        scopes: vec![
            ScopeCount {
                scope: Scope::AllMail,
                hits: 14,
            },
            ScopeCount {
                scope: Scope::Inbox,
                hits: 6,
            },
            ScopeCount {
                scope: Scope::Lists,
                hits: 8,
            },
        ],
        refinements: vec![
            refinement("is:unread", 9),
            refinement("larger:1M", 5),
            refinement("is:flagged", 2),
            refinement("in:lkml", 8),
            // Nothing is a draft, so this one leads nowhere and must not be
            // drawn.
            refinement("is:draft", 0),
        ],
    }
}

fn refinement(token: &str, hits: u64) -> Refinement {
    Refinement {
        token: token.to_owned(),
        hits,
    }
}

/// Whether the folder list is on screen — the thing the column displaces.
///
/// By widget type, not by CSS class: the shell's sidebar *pane* wears
/// `.postio-sidebar` too and never goes away, so a class lookup finds the
/// container rather than what is in it.
fn sidebar_shows_folders(window: &Window) -> bool {
    find(&window.clone().upcast(), &|widget| {
        widget.type_().name() == "PostioSidebar"
    })
    .map(|sidebar| sidebar.property::<bool>("visible"))
    .unwrap_or(false)
}

/// The scope rows as `(name, count)`, read off the widgets.
fn scope_rows(window: &Window) -> Vec<(String, String)> {
    let list = scope_list(window);
    let mut rows = Vec::new();
    let mut index = 0;
    while let Some(row) = list.row_at_index(index) {
        let line = row.child().expect("a row has a line");
        let name = line.first_child().expect("a name");
        let count = line.last_child().expect("a count");
        rows.push((label_text(&name), label_text(&count)));
        index += 1;
    }
    rows
}

/// Picks a scope the way a click does, so the panel treats it as a choice.
fn pick_scope(window: &Window, index: i32) {
    let list = scope_list(window);
    let row = list.row_at_index(index).expect("that scope exists");
    list.select_row(Some(&row));
}

fn scope_list(window: &Window) -> gtk::ListBox {
    let panel = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-search-panel")
    })
    .expect("the column is mounted");
    find(&panel, &|widget| widget.has_css_class("postio-folders"))
        .expect("the column has a scope list")
        .downcast()
        .expect("it is a list box")
}

/// Activates a refine chip the way a click or `Enter` on it does.
fn click_chip(window: &Window, token: &str) {
    let chip = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-refine-chip")
            && widget
                .clone()
                .downcast::<gtk::Button>()
                .ok()
                .and_then(|button| button.label())
                .is_some_and(|label| label == token)
    })
    .unwrap_or_else(|| panic!("a chip labelled {token}"));
    chip.downcast::<gtk::Button>()
        .expect("a chip is a button")
        .emit_clicked();
}

fn refine_note(window: &Window) -> String {
    find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-refine-empty")
    })
    .map(|label| label_text(&label))
    .unwrap_or_default()
}

fn focused_is_a_chip(window: &Window) -> bool {
    let mut widget = gtk::prelude::GtkWindowExt::focus(window.upcast_ref::<gtk::Window>());
    while let Some(current) = widget {
        if current.has_css_class("postio-refine-chip") {
            return true;
        }
        widget = current.parent();
    }
    false
}

fn label_text(widget: &gtk::Widget) -> String {
    widget
        .clone()
        .downcast::<gtk::Label>()
        .map(|label| label.text().to_string())
        .unwrap_or_default()
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
