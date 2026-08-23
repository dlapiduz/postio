//! The one box on a real display: three modes, one field.
//!
//! Replaces `gtk_palette.rs` and `gtk_search.rs`, which tested two surfaces
//! that are now one. Skips without a display. Nothing here touches the
//! network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Context};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::shell::Pane;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

#[test]
fn one_box_searches_mail_runs_commands_and_jumps_to_folders() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    let finder = window.finder();
    finder.set_mailboxes(&folders());

    let ran: Rc<RefCell<Vec<CommandId>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_command({
        let ran = ran.clone();
        move |id| ran.borrow_mut().push(id)
    });
    let searched: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    finder.connect_search({
        let searched = searched.clone();
        move |parsed| searched.borrow_mut().push(parsed.input().to_string())
    });
    let jumped: Rc<RefCell<Vec<MailboxId>>> = Rc::new(RefCell::new(Vec::new()));
    finder.connect_folder({
        let jumped = jumped.clone();
        move |id| jumped.borrow_mut().push(id)
    });

    // ── closed, it is the field the canvas draws at rest ─────────────────
    assert!(!finder.is_open());
    assert_eq!(finder.context(), None, "a closed box owns no keyboard");

    // ── `/` opens it on mail ─────────────────────────────────────────────
    window.open_finder(Mode::Search);
    pump();
    assert!(finder.is_open(), "/ opens the box");
    assert_eq!(finder.mode(), Mode::Search);
    assert_eq!(finder.context(), Some(Context::Search));
    assert!(
        !finder.is_visible(),
        "search answers in the message list, not on a plate"
    );

    finder.set_query(Query {
        mode: Mode::Search,
        text: "from:ada@example.com report".into(),
    });
    pump();
    let chips = finder.chips();
    assert_eq!(chips.len(), 1, "one operator, one chip");
    assert_eq!(chips[0].label, "from:ada@example.com");
    assert!(chips[0].complete, "free text alongside it stays plain");

    finder.activate();
    assert_eq!(
        searched.borrow().len(),
        1,
        "Enter in search mode runs the search"
    );

    // ── `>` turns the same box into the command palette ──────────────────
    // Typed, not set: absorbing the prefix is the behaviour under test.
    finder.set_query(Query::new());
    type_into(&window, ">");
    pump();
    assert_eq!(finder.mode(), Mode::Command, "> switches the box");
    assert_eq!(finder.query().text, "", "and the prefix left the text");
    assert_eq!(finder.context(), Some(Context::Palette));
    assert!(finder.is_visible(), "a mode with results shows them");
    assert!(
        !finder.commands().is_empty(),
        "an empty command query offers everything applicable"
    );

    type_into(&window, "archiv");
    pump();
    assert_eq!(
        finder.commands().first(),
        Some(&CommandId::Archive),
        "fuzzy matching puts the obvious answer first: {:?}",
        finder.commands()
    );
    finder.activate();
    pump();
    assert_eq!(*ran.borrow(), vec![CommandId::Archive]);
    assert!(!finder.is_open(), "running a command closes the box");

    // ── nothing matched is never a shrug ─────────────────────────────────
    window.open_finder(Mode::Command);
    type_into(&window, "zzzzz");
    pump();
    assert!(finder.commands().is_empty());
    finder.activate();
    assert_eq!(
        *ran.borrow(),
        vec![CommandId::Archive],
        "and Enter runs nothing"
    );

    // ── Backspace at the start gives the mode back, keeping the words ────
    finder.set_query(Query {
        mode: Mode::Command,
        text: "arch".into(),
    });
    caret_to_start(&window);
    assert!(finder.press_backspace(), "there is a mode to back out of");
    assert_eq!(finder.mode(), Mode::Search);
    assert_eq!(
        finder.query().text,
        "arch",
        "backing out of a mode should not cost what was typed"
    );

    // ── `#` jumps to a folder ────────────────────────────────────────────
    finder.set_query(Query::new());
    type_into(&window, "#wd");
    pump();
    assert_eq!(finder.mode(), Mode::Mailbox);
    assert_eq!(
        finder.folders(),
        vec![MailboxId::new(3)],
        "`wd` finds wayland-devel, the way `cp` finds a command"
    );
    finder.activate();
    pump();
    assert_eq!(*jumped.borrow(), vec![MailboxId::new(3)]);

    // ── Esc closes it and gives the keyboard back ────────────────────────
    window.shell().set_focused_pane(Pane::Reader);
    window.open_finder(Mode::Command);
    pump();
    assert!(finder.is_open());
    window.close_finder();
    pump();
    assert!(!finder.is_open(), "Esc closes the box");
    assert_eq!(
        window.shell().focused_pane(),
        Pane::Reader,
        "and puts the keyboard back where it was found"
    );
    assert_eq!(finder.query(), Query::new(), "and leaves nothing behind");

    window.destroy();
}

/// Canvas 1b's folders, for `#` to find.
fn folders() -> Vec<Mailbox> {
    let folder = |id: i64, path: &str, role, unread| {
        let mut mailbox = Mailbox::new(AccountId::new(1), path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total: 100,
            unread,
            flagged: 0,
        };
        mailbox
    };
    vec![
        folder(1, "INBOX", MailboxRole::Inbox, 12),
        folder(2, "Archive", MailboxRole::Archive, 0),
        folder(3, "wayland-devel", MailboxRole::Regular, 37),
    ]
}

/// Type `text` into the header's field, one insertion, as a user would.
fn type_into(window: &Window, text: &str) {
    let entry = field(window);
    let position = entry.text().len() as i32;
    entry.set_text(&format!("{}{text}", entry.text()));
    entry.set_position(position + text.len() as i32);
}

fn caret_to_start(window: &Window) {
    field(window).set_position(0);
}

fn field(window: &Window) -> gtk::Text {
    fn find(widget: &gtk::Widget) -> Option<gtk::Text> {
        if let Some(text) = widget.downcast_ref::<gtk::Text>() {
            return Some(text.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(window.upcast_ref::<gtk::Widget>()).expect("the header's one box")
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..80 {
        while context.iteration(false) {}
    }
}
