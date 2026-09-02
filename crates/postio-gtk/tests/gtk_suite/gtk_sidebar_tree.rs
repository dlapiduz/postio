//! Subfolders in the sidebar (#324): nested, collapsible, and a `\Noselect`
//! parent that can only toggle. On a real display, because a disclosure
//! click has to reach an actual `GtkButton`'s `clicked` signal.
//!
//! One test function, for the reason `gtk_sidebar_keys.rs` gives. Skips
//! without a display. Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. Set before the app under test starts, which is
// the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::sidebar::Sidebar;
use postio_gtk::state::SidebarState;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::Mailbox;

fn pump() {
    for _ in 0..80 {
        glib::MainContext::default().iteration(false);
    }
}

/// `Clients` (a real folder) and `Lists` (`\Noselect`, organizing two
/// children only), each with children of their own.
fn hierarchy() -> Vec<Mailbox> {
    let account = AccountId::new(1);
    let folder = |id: i64, parent: Option<i64>, path: &str, selectable: bool| {
        let mut mailbox = Mailbox::new(account, path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.parent_id = parent.map(MailboxId::new);
        mailbox.selectable = selectable;
        mailbox
    };
    vec![
        folder(1, None, "Clients", true),
        folder(2, Some(1), "Clients/Acme", true),
        folder(3, None, "Lists", false),
        folder(4, Some(3), "Lists/postio-devel", true),
        folder(5, Some(3), "Lists/other-project", true),
    ]
}

/// Every ordinary-section row currently drawn, in order.
fn tree_rows(sidebar: &Sidebar) -> Vec<gtk::ListBoxRow> {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-folder-tree")
        .into_iter()
        .filter_map(|w| w.downcast().ok())
        .collect()
}

fn row_names(sidebar: &Sidebar) -> Vec<String> {
    tree_rows(sidebar)
        .iter()
        .map(|row| {
            let widget = row.clone().upcast::<gtk::Widget>();
            let name: gtk::Label = collect(&widget, "postio-folder-name")[0]
                .clone()
                .downcast()
                .unwrap();
            name.text().to_string()
        })
        .collect()
}

fn disclosure_of(row: &gtk::ListBoxRow) -> gtk::Button {
    collect(
        &row.clone().upcast::<gtk::Widget>(),
        "postio-folder-disclosure",
    )[0]
    .clone()
    .downcast()
    .unwrap()
}

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

pub fn folders_nest_collapse_and_a_noselect_parent_only_toggles() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-sidebar-tree-{}", std::process::id()));
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

    // ── nested, indented, and open by default ──────────────────────────
    let sidebar = Sidebar::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&sidebar));
    window.set_default_size(212, 700);
    window.present();
    pump();

    sidebar.set_mailboxes(&hierarchy());
    pump();

    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Acme", "Lists", "other-project", "postio-devel"],
        "a fresh account starts fully open, same folders a flat list already \
         showed, just nested correctly now"
    );

    // ── the disclosure collapses and reopens its own children only ──────
    let changed: Rc<RefCell<u32>> = Default::default();
    sidebar.connect_collapsed_changed({
        let changed = Rc::clone(&changed);
        move || *changed.borrow_mut() += 1
    });

    let clients_row = tree_rows(&sidebar)[0].clone();
    disclosure_of(&clients_row).emit_clicked();
    pump();
    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Lists", "other-project", "postio-devel"],
        "collapsing Clients hides Acme and nothing else"
    );
    assert_eq!(*changed.borrow(), 1);

    disclosure_of(&clients_row).emit_clicked();
    pump();
    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Acme", "Lists", "other-project", "postio-devel"],
        "clicking it again reopens Clients"
    );

    // ── a `\Noselect` parent can only toggle ─────────────────────────────
    let opened: Rc<RefCell<Vec<i64>>> = Default::default();
    sidebar.connect_selected({
        let opened = Rc::clone(&opened);
        move |id| opened.borrow_mut().push(id.get())
    });

    let lists_row = tree_rows(&sidebar)[2].clone();
    assert_eq!(row_names(&sidebar)[2], "Lists");
    let list: gtk::ListBox = lists_row.parent().and_then(|p| p.downcast().ok()).unwrap();
    list.select_row(Some(&lists_row));
    pump();
    assert!(
        opened.borrow().is_empty(),
        "a `\\Noselect` folder has nothing to open"
    );
    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Acme", "Lists"],
        "selecting it toggled instead — Lists started open, so this click \
         on the row itself closed it, same as the disclosure would"
    );
    // Reopen it: the next section wants both branches visible. `toggle`
    // directly, not another click on the same already-selected row — GTK
    // does not re-fire `row-selected` for a row selecting itself again.
    sidebar.toggle(MailboxId::new(3));
    pump();
    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Acme", "Lists", "other-project", "postio-devel"]
    );

    // ── selecting a folder under a collapsed parent reveals it ──────────
    disclosure_of(&clients_row).emit_clicked(); // collapse Clients again
    pump();
    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Lists", "other-project", "postio-devel"]
    );

    sidebar.select(MailboxId::new(2)); // Acme, hidden right now
    pump();
    assert_eq!(
        row_names(&sidebar),
        ["Clients", "Acme", "Lists", "other-project", "postio-devel"],
        "opening a folder whose parent is closed must open the parent too, \
         or the open folder would show as nothing selected at all (#324)"
    );
    assert_eq!(sidebar.selected(), Some(MailboxId::new(2)));

    window.destroy();

    // ── which folders are collapsed survives a restart ───────────────────
    // A fresh `Window`, reading the same `XDG_STATE_HOME`, is the closest
    // thing to a restart this process can do.
    let first = Window::default();
    first.present();
    pump();
    first.sidebar().set_mailboxes(&hierarchy());
    pump();
    first.sidebar().toggle(MailboxId::new(1)); // collapse Clients
    pump();
    assert_eq!(
        SidebarState::load().collapsed_folders,
        HashSet::from([MailboxId::new(1)]),
        "toggling should have saved immediately, not waited for a clean exit"
    );
    first.destroy();

    let second = Window::default();
    second.present();
    pump();
    assert_eq!(
        second.sidebar().collapsed(),
        HashSet::from([MailboxId::new(1)]),
        "a new window should restore what the last one closed"
    );
    second.sidebar().set_mailboxes(&hierarchy());
    pump();
    assert_eq!(
        row_names(&second.sidebar()),
        ["Clients", "Lists", "other-project", "postio-devel"],
        "Clients should still render collapsed"
    );

    // Reachable through the real keyboard path too, not only the API:
    // `g f` into the sidebar, `j` to actually select Clients (`g f` alone
    // only focuses — see `Sidebar::focus_folders`), then `space` reopens it.
    let opened_via_keyboard: Rc<RefCell<Vec<i64>>> = Default::default();
    second.sidebar().connect_selected({
        let opened = Rc::clone(&opened_via_keyboard);
        move |id| opened.borrow_mut().push(id.get())
    });
    press(&second, "g");
    press(&second, "f");
    assert_eq!(second.context(), Context::Sidebar);
    press(&second, "j");
    assert_eq!(*opened_via_keyboard.borrow(), vec![1], "landed on Clients");
    press(&second, "space");
    pump();
    assert_eq!(
        row_names(&second.sidebar()),
        ["Clients", "Acme", "Lists", "other-project", "postio-devel"],
        "`space` on the focused folder toggled it open, via the registry \
         command rather than a hard-coded key"
    );

    second.destroy();
}

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();
}
