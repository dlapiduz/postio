//! The reading pane's action bar (#498) on a real display: it appears
//! whenever a message is open (rendered, or still explaining why it has no
//! body), hides when the pane is genuinely empty, carries the live keymap's
//! keys rather than hard-coded letters, and each button runs the exact
//! `Command` the keybinding for the same verb would.
//!
//! `src/reader/actions.rs`'s own `#[cfg(test)]` module proves the pure
//! key-lookup logic without a display; what needs one here is that the
//! widgets actually reflect it and that a click reaches a `connect_command`
//! handler, which is the same claim `list_view`'s row actions are proven by.
//!
//! One test function: GTK is single-threaded and initialised once per
//! process. Skips without a display.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{Command, CommandId, Keymap};
use postio_gtk::reader::{Absent, BlobSource, Reader, RemoteImageAllowList};
use postio_model::message::MessageBody;

fn pump() {
    for _ in 0..40 {
        glib::MainContext::default().iteration(false);
    }
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "postio-gtk-reader-actions-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.ini"))
}

/// Depth-first search of a widget tree for the first one carrying `class`.
fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn button(root: &gtk::Widget, class: &str) -> gtk::Button {
    find(root, class)
        .unwrap_or_else(|| panic!("no widget carries {class}"))
        .downcast()
        .expect("the action bar's own buttons are gtk::Button")
}

fn hint_text(root: &gtk::Widget, class: &str) -> Option<String> {
    let label: gtk::Label = find(root, &format!("{class}-hint"))?
        .downcast()
        .expect("the hint is a gtk::Label");
    label.get_visible().then(|| label.label().to_string())
}

#[test]
fn the_action_bar_follows_the_pane_carries_the_keymap_and_runs_registry_commands() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }

    let source: Rc<dyn BlobSource> = Rc::new(|_: &str| None);
    let reader = Reader::with_allowlist(
        source,
        RemoteImageAllowList::default(),
        scratch_path("allowlist"),
    );

    let window = gtk::Window::new();
    window.set_default_size(700, 500);
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();

    let root = reader.widget();

    // ── absent by default: nothing has ever been selected ─────────────────
    assert!(
        !button(&root, "postio-reader-action-reply").is_visible(),
        "the bar must not show before anything is open"
    );

    // ── a rendered message shows it ────────────────────────────────────────
    let body = MessageBody {
        text: Some("Half past twelve?".to_owned()),
        html: None,
    };
    reader.render(&body, None);
    pump();
    assert!(
        button(&root, "postio-reader-action-reply").is_visible(),
        "a rendered message is open; the bar must show"
    );

    // ── still open while the body has not arrived yet ──────────────────────
    reader.show_absent(Absent::Partial);
    pump();
    assert!(
        button(&root, "postio-reader-action-reply").is_visible(),
        "headers are here even though the body is not -- the message is \
         still open, so Reply/Forward/Archive stay reachable"
    );

    // ── and hides again once the pane is genuinely empty ───────────────────
    reader.clear();
    pump();
    assert!(
        !button(&root, "postio-reader-action-reply").is_visible(),
        "nothing selected: the bar must not show"
    );

    // ── the default keys match the registry ────────────────────────────────
    reader.render(&body, None);
    pump();
    assert_eq!(
        hint_text(&root, "postio-reader-action-reply").as_deref(),
        Some("e")
    );
    assert_eq!(
        hint_text(&root, "postio-reader-action-reply-all").as_deref(),
        Some("E")
    );
    assert_eq!(
        hint_text(&root, "postio-reader-action-forward").as_deref(),
        Some("f")
    );
    assert_eq!(
        hint_text(&root, "postio-reader-action-archive").as_deref(),
        Some("a")
    );

    // ── a rebind reaches the button, not just the resolver ─────────────────
    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("reply".to_string(), "r".to_string());
    reader.set_keymap(&Keymap::resolve(&overrides));
    pump();
    assert_eq!(
        hint_text(&root, "postio-reader-action-reply").as_deref(),
        Some("r"),
        "config.toml's [keys] must reach the pointer's hint, not just the keyboard"
    );

    // ── every button runs the same Command the keyboard would ──────────────
    let seen: Rc<RefCell<Vec<Command>>> = Rc::new(RefCell::new(Vec::new()));
    reader.connect_command({
        let seen = Rc::clone(&seen);
        move |command| seen.borrow_mut().push(command)
    });

    for (class, id) in [
        ("postio-reader-action-reply", CommandId::Reply),
        ("postio-reader-action-reply-all", CommandId::ReplyAll),
        ("postio-reader-action-forward", CommandId::Forward),
        ("postio-reader-action-archive", CommandId::Archive),
    ] {
        button(&root, class).emit_clicked();
        pump();
        assert_eq!(
            seen.borrow().last(),
            Some(&Command::default_for(id)),
            "{class} must run the same invocation the {id} keybinding does"
        );
    }
    assert_eq!(seen.borrow().len(), 4);
}
