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

use crate::settle;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Context};
use postio_gtk::finder::Mode;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxRole};

/// How long a wait below may go without seeing its condition before it is
/// called a hang.
///
/// A *liveness* bound, not a latency claim: the waits are already
/// event-driven (real watcher, real debounce), so this deadline does no
/// measuring -- it only turns a genuine stall into a failure with a name.
/// Deliberately enormous, the way `postio-core`'s own config-watcher test
/// fixed the identical shape (#219): at 5 seconds this flaked on a pristine
/// `main` checkout under nothing more than ordinary shared-box contention
/// (#838). See `docs/engineering-notes.md`'s "tests that fail under load"
/// doctrine -- "liveness deadlines are minutes, not budgets."
const PATIENCE: Duration = Duration::from_secs(120);

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
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

/// Saved-search names in the order the sidebar actually draws them --
/// [`saved_search_names`] sorts, which would hide a reorder.
fn saved_search_names_in_order(window: &Window) -> Vec<String> {
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
    out
}

/// #455: `j`/`k` used to stop at the last folder -- a saved search had no
/// keyboard path into it at all, so its rename/move/delete verbs had
/// nowhere to bind a key to either. This is the round trip a keyboard user
/// now makes: cross into the saved searches the same way `gtk_sidebar_keys
/// .rs` proves crossing between the two folder sections, run the one landed
/// on the same way a click would, and reorder it without touching the
/// mouse -- end to end, through the real `config.toml` `install_at` writes
/// to and the sidebar it repaints from, the same shape
/// `pinned_filters_reach_the_sidebar_and_ctrl_s_adds_one` already uses.
pub fn keyboard_reaches_saved_searches_and_their_move_verbs() {
    let root =
        std::env::temp_dir().join(format!("postio-saved-searches-keys-{}", std::process::id()));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let config_dir = root.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let path = config_dir.join("config.toml");
    // Nothing pinned yet, so `j`/`k` below has something else to cross
    // *from*: the one folder set a moment later. (A pin already on disk at
    // `present()` time no longer needs sidestepping here -- see #614's own
    // `a_pinned_saved_search_does_not_auto_run_on_first_present`.)
    std::fs::write(&path, "").unwrap();

    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    settle();

    // One folder, so there is somewhere for `j` to cross from -- the same
    // gap #455 names: without it, the folder list and the saved searches
    // could never be proven to share one `j`/`k` idiom.
    let account = AccountId::new(1);
    let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
    inbox.id = MailboxId::new(1);
    inbox.role = MailboxRole::Inbox;
    window.sidebar().set_mailboxes(&[inbox]);
    settle();

    // Pin both searches by editing the file underneath the running window,
    // the same live path `pinned_filters_reach_the_sidebar_and_ctrl_s_adds_one`
    // proves -- by now the window's initial focus has already settled on
    // the one folder, so populating the searches afterward cannot steal it.
    std::fs::write(
        &path,
        "[filters.alpha]\nquery = \"subject:alpha\"\npinned = true\n\n\
         [filters.beta]\nquery = \"subject:beta\"\npinned = true\n",
    )
    .unwrap();
    assert!(
        wait_until(
            || saved_search_names_in_order(&window) == ["alpha".to_string(), "beta".to_string()]
        ),
        "the pinned searches never reached the sidebar: got {:?}",
        saved_search_names_in_order(&window)
    );

    // Every command the window actually ran, so the guard below can prove a
    // negative (nothing fired) as well as the positive.
    let commands: Rc<RefCell<Vec<CommandId>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_command({
        let commands = commands.clone();
        move |id| commands.borrow_mut().push(id)
    });

    // ── `g f`, then `j` crosses from the one folder into the searches ─────
    press(&window, "g", gdk::ModifierType::empty());
    press(&window, "f", gdk::ModifierType::empty());
    assert_eq!(window.context(), Context::Sidebar);
    press(&window, "j", gdk::ModifierType::empty()); // onto INBOX
    press(&window, "j", gdk::ModifierType::empty()); // onto alpha

    let finder = window.finder();
    assert!(
        finder.is_open(),
        "stepping onto a saved search should run it, exactly as a click does"
    );
    assert_eq!(
        finder.query().text,
        "subject:alpha",
        "`j` landed on the wrong row, or did not run it"
    );
    // `window.close_finder()`, not `finder.close()`: the latter only closes
    // the widget, while the former also gives the keyboard back to
    // `Context::Sidebar` -- the same restoration `Esc` does, and what the
    // shift+Up/Down presses below need to still resolve in.
    window.close_finder();
    settle();
    assert_eq!(window.context(), Context::Sidebar);
    // `close_finder` restores `Context::Sidebar` above, and must also give
    // real GTK keyboard focus back to the row that opened the search --
    // `before_finder`'s own notion of "pane" predates the sidebar being a
    // keyboard context at all (`postio-cfd.2`) and has no shape to name one
    // of its rows, so `close_finder` asks the sidebar directly the same way
    // `enter_sidebar` already does (#614). Without that, the shift+Down
    // below would resolve against whatever `shell().grab_focus()`'s own
    // memory happened to restore instead.
    let focused = gtk::prelude::GtkWindowExt::focus(&window);
    let alpha = saved_search_row(&window, "alpha").expect("alpha's row");
    assert_eq!(
        focused.as_ref(),
        Some(alpha.upcast_ref::<gtk::Widget>()),
        "closing a search opened by stepping onto a saved search should return \
         real keyboard focus to that row"
    );

    // ── `shift+Down` moves the focused saved search down, on disk and on
    //    screen, with no dialog to answer -- unlike rename/delete, which
    //    this test leaves to the mouse-driven
    //    `the_context_menu_reaches_the_action_handler_with_the_right_key`
    //    since routing the same four verbs through the registry, proved
    //    here, is what was actually missing (#455) ────────────────────────
    press(&window, "Down", gdk::ModifierType::SHIFT_MASK);
    assert!(
        commands.borrow().contains(&CommandId::MoveSavedSearchDown),
        "shift+Down should have resolved to move_saved_search_down in Context::Sidebar"
    );
    assert_eq!(
        saved_search_names_in_order(&window),
        ["beta".to_string(), "alpha".to_string()],
        "moving the focused saved search down should reorder it on screen"
    );
    // Persisted order lives in each filter's own `order` field, not in the
    // TOML's textual section order (`toml`'s writer emits tables
    // alphabetically regardless) -- `ordered_filter_keys` is the same read
    // `saved_searches` uses, so this is the reorder surviving a restart,
    // not just staying on screen for the rest of this process.
    let reloaded =
        postio_config::Config::load_from_path(&path).expect("the reordered file should parse");
    assert_eq!(
        reloaded.ordered_filter_keys(),
        vec!["beta".to_string(), "alpha".to_string()],
        "the reorder must reach disk, or it would not survive a restart"
    );

    // ── the same key over a folder does nothing: the guard holds (#455) ────
    // `Context::Sidebar` covers folder rows and saved-search rows alike, so
    // `MoveSavedSearchDown` still *resolves* here -- it is
    // `Sidebar::focused_saved_search` returning `None` for a folder row,
    // not the registry, that has to stop it from acting on one.
    let before_guard = saved_search_names_in_order(&window);
    press(&window, "k", gdk::ModifierType::empty()); // back onto INBOX
    press(&window, "Down", gdk::ModifierType::SHIFT_MASK);
    assert_eq!(
        saved_search_names_in_order(&window),
        before_guard,
        "a folder was focused, not a saved search -- this must not touch the file"
    );

    window.close();
    settle();
    let _ = std::fs::remove_dir_all(&root);
}

/// #614, symptom 1: a pinned filter is on screen before the window ever
/// presents (`install_at` runs first, same as `app.rs`'s own startup
/// order), and with no folders synced yet its saved-search row is the only
/// focusable thing in the sidebar. `GtkListBox` in `SelectionMode::Single`
/// selects a row it focuses, and selecting a saved search runs it -- so an
/// undirected initial focus can open the app straight into a search result
/// instead of the message list, on a fresh install or any account that is
/// still syncing.
pub fn a_pinned_saved_search_does_not_auto_run_on_first_present() {
    let root = std::env::temp_dir().join(format!(
        "postio-saved-searches-startup-{}",
        std::process::id()
    ));
    let state_dir = root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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

    // Exactly `app.rs`'s own order: build, load config (which puts the
    // pinned row on screen), then present -- and no mailboxes at all, the
    // still-syncing shape that leaves the saved search as the only
    // focusable row in the sidebar.
    let window = Window::default();
    postio_gtk::config::install_at(&window, &path);
    window.present();
    settle();

    assert_eq!(
        saved_search_names(&window),
        ["needs-reply".to_string()],
        "the pinned filter should be on screen for this to be a real test of it"
    );
    assert!(
        !window.finder().is_open(),
        "a pinned saved search must not auto-select and run itself just because \
         the window presented with nothing else in the sidebar to focus"
    );
    assert_eq!(
        window.context(),
        Context::List,
        "the window should still present onto the message list by default"
    );

    window.close();
    settle();
    let _ = std::fs::remove_dir_all(&root);
}
