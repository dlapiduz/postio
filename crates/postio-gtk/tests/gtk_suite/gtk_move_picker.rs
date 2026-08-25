//! `m` asks where to move to, and the answer becomes the move.
//!
//! `postio-agr.2`. `Command::Move { to: None }` means "ask the user" by
//! design — see `Command`'s docs — and the box already has a folder mode, so
//! the missing piece was only ever the keyboard route between them: open the
//! picker, and turn the folder that comes back into the move that was pending.
//!
//! The interesting property is the one the bead names last: a move the user
//! abandoned must not be answered by an unrelated folder jump later. Skips
//! without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_core::{Command, MessageTarget};
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::finder::{Mode, Query};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const ACCOUNT: i64 = 1;
const INBOX: i64 = 1;
const WAYLAND: i64 = 3;

#[test]
fn m_opens_the_folder_picker_and_the_folder_picked_becomes_the_move() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    let store = Rc::new(Store);
    let _feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "lena@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let acted: Rc<RefCell<Vec<Command>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_action({
        let acted = acted.clone();
        move |command| acted.borrow_mut().push(command)
    });
    let finder = window.finder();
    finder.set_mailboxes(&folders());

    // ── `m` asks rather than guessing ────────────────────────────────────
    window.act(Command::Move {
        target: MessageTarget::Selection,
        to: None,
    });
    pump();

    assert!(finder.is_open(), "`m` opens the box");
    assert_eq!(finder.mode(), Mode::Mailbox, "on folders, not on mail");
    assert!(
        acted.borrow().is_empty(),
        "a move with no destination is half a request; nothing has moved yet"
    );

    // ── the folder picked becomes the move ───────────────────────────────
    type_into(&window, "wd");
    pump();
    assert_eq!(finder.folders(), vec![MailboxId::new(WAYLAND)]);
    finder.activate();
    pump();

    assert_eq!(
        *acted.borrow(),
        vec![Command::Move {
            target: MessageTarget::Selection,
            to: Some(MailboxId::new(WAYLAND)),
        }],
        "the same command a drop produces, so it is undoable the same way \
         and reaches the server through the same queue"
    );
    assert_eq!(
        window.sidebar().selected(),
        Some(MailboxId::new(INBOX)),
        "picking somewhere to move to is not going there"
    );

    // ── Esc cancels without moving anything ──────────────────────────────
    acted.borrow_mut().clear();
    window.act(Command::Move {
        target: MessageTarget::Selection,
        to: None,
    });
    pump();
    window.close_finder();
    pump();

    assert!(!finder.is_open());
    assert!(acted.borrow().is_empty(), "Esc moves nothing");

    // ── and the abandoned move does not answer a later `#` jump ──────────
    window.open_finder(Mode::Mailbox);
    finder.set_query(Query::new());
    type_into(&window, "#wd");
    pump();
    finder.activate();
    pump();

    assert!(
        acted.borrow().is_empty(),
        "the pending move died with the box it was asked in"
    );
    assert_eq!(
        window.sidebar().selected(),
        Some(MailboxId::new(WAYLAND)),
        "`#` jumped, which is what `#` is for"
    );

    window.destroy();
}

/// Folders for `#` to find. The list is never read from here — this test is
/// about where a move goes, not about what a page holds.
struct Store;

fn folders() -> Vec<Mailbox> {
    let folder = |id: i64, path: &str, role| {
        let mut mailbox = Mailbox::new(AccountId::new(ACCOUNT), path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total: 100,
            unread: 0,
            flagged: 0,
        };
        mailbox
    };
    vec![
        folder(INBOX, "INBOX", MailboxRole::Inbox),
        folder(2, "Archive", MailboxRole::Archive),
        folder(WAYLAND, "wayland-devel", MailboxRole::Regular),
    ]
}

impl MailboxSource for Store {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        Box::pin(async move { Ok(folders()) })
    }
}

impl MessageSource for Store {
    fn fetch(&self, _request: PageRequest) -> PageFuture {
        Box::pin(async move {
            Ok(Page {
                total: 0,
                rows: Vec::new(),
            })
        })
    }
}

/// Type `text` into the header's field, one insertion, as a user would.
fn type_into(window: &Window, text: &str) {
    let entry = field(window);
    let position = entry.text().len() as i32;
    entry.set_text(&format!("{}{text}", entry.text()));
    entry.set_position(position + text.len() as i32);
}

fn field(window: &Window) -> gtk::Text {
    fn find(widget: &gtk::Widget) -> Option<gtk::Text> {
        if let Some(text) = widget.downcast_ref::<gtk::Text>() {
            return Some(text.clone());
        }
        let mut child = widget.first_child();
        while let Some(node) = child {
            if let Some(found) = find(&node) {
                return Some(found);
            }
            child = node.next_sibling();
        }
        None
    }
    find(window.upcast_ref::<gtk::Widget>()).expect("the header has a field")
}

fn pump() {
    let context = glib::MainContext::default();
    for _ in 0..40 {
        while context.iteration(false) {}
    }
}
