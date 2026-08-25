//! Issue #10, end to end: pinned `[filters]` on disk reach the sidebar at
//! startup and live, activating one runs its query, and `Ctrl+S` writes a
//! new one back out.
//!
//! Same shape as `gtk_live_config.rs`: a real `config.toml`, a real watcher,
//! nothing mocked. `Config::save_filter`'s naming and uniqueness are unit
//! tested in `postio-config`; this is the wiring those tests cannot see --
//! that `postio-gtk/src/config.rs::install_at` actually calls it, actually
//! writes the file, and actually repaints the sidebar, both from its own
//! `Ctrl+S` and from a hand edit to the file underneath it.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::Mode;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

const PATIENCE: Duration = Duration::from_secs(5);

fn wait_until(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn saved_search_names(window: &Window) -> Vec<String> {
    fn collect(widget: &gtk::Widget, out: &mut Vec<String>) {
        if widget.has_css_class("postio-saved-search")
            && let Some(row) = widget.clone().downcast::<gtk::ListBoxRow>().ok()
            && let Some(label) = row.child().and_then(|c| c.downcast::<gtk::Label>().ok())
        {
            out.push(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect(&current, out);
            child = current.next_sibling();
        }
    }
    let mut out = Vec::new();
    collect(window.sidebar().upcast_ref::<gtk::Widget>(), &mut out);
    out.sort();
    out
}

/// The saved-search row labelled `name`, if the sidebar is showing one.
fn saved_search_row(window: &Window, name: &str) -> Option<gtk::ListBoxRow> {
    fn find(widget: &gtk::Widget, name: &str) -> Option<gtk::ListBoxRow> {
        if widget.has_css_class("postio-saved-search")
            && let Some(row) = widget.clone().downcast::<gtk::ListBoxRow>().ok()
            && let Some(label) = row.child().and_then(|c| c.downcast::<gtk::Label>().ok())
            && label.text() == name
        {
            return Some(row);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current, name) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(window.sidebar().upcast_ref::<gtk::Widget>(), name)
}

pub fn pinned_filters_reach_the_sidebar_and_ctrl_s_adds_one() {
    let root = std::env::temp_dir().join(format!("postio-saved-searches-{}", std::process::id()));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let path = config_dir.join("config.toml");
    std::fs::write(
        &path,
        "[filters.needs-reply]\nquery = \"is:unread from:team\"\npinned = true\n",
    )
    .unwrap();

    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    settle();

    // ── what was pinned on disk at startup is on screen ────────────────────
    assert_eq!(
        saved_search_names(&window),
        ["needs-reply".to_string()],
        "a pinned filter already on disk should not need a restart to show"
    );

    // ── activating the row runs its query, through install_at's own wiring ─
    let row = saved_search_row(&window, "needs-reply").expect("the pinned filter's row");
    let list: gtk::ListBox = row.parent().and_then(|p| p.downcast().ok()).unwrap();
    list.select_row(Some(&row));
    settle();

    let finder = window.finder();
    assert!(
        finder.is_open(),
        "picking a saved search should open the box"
    );
    assert_eq!(finder.mode(), Mode::Search);
    assert_eq!(finder.query().text, "is:unread from:team");
    finder.close();
    settle();

    // ── Ctrl+S while a query is typed saves it and shows it right away ─────
    window.run_search("has:attach");
    settle();
    window.handle_key(
        gdk::Key::from_name("s").unwrap(),
        gdk::ModifierType::CONTROL_MASK,
    );
    settle();

    assert_eq!(
        saved_search_names(&window),
        ["has-attach".to_string(), "needs-reply".to_string()],
        "saving must show the new filter immediately, not wait for the watcher"
    );

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("[filters.has-attach]") && saved.contains("query = \"has:attach\""),
        "the save must reach disk, or it would not survive a restart:\n{saved}"
    );

    // ── and a hand edit reaches the sidebar too, with no restart ───────────
    std::fs::write(
        &path,
        "[filters.needs-reply]\nquery = \"is:unread from:team\"\npinned = true\n\n\
         [filters.has-attach]\nquery = \"has:attach\"\npinned = true\n\n\
         [filters.from-ada]\nquery = \"from:ada\"\npinned = true\n",
    )
    .unwrap();
    assert!(
        wait_until(|| saved_search_names(&window)
            == ["from-ada", "has-attach", "needs-reply"]
                .map(str::to_string)
                .to_vec()),
        "a hand edit to [filters] never reached the sidebar: got {:?}",
        saved_search_names(&window)
    );

    window.close();
    settle();
    let _ = std::fs::remove_dir_all(&root);
}
