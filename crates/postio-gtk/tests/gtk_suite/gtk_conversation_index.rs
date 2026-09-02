//! The drill-in column is an index into the conversation pane (#308).
//!
//! ADR 0015 Q4's resolution: both surfaces survive because they do different
//! jobs. The column lists a line per message and **jumps**; the pane holds
//! the conversation. There is one current message and moving either surface
//! moves the other.
//!
//! Two things are worth a display test and neither is obvious from the
//! widgets alone:
//!
//! * **They do not ring.** Each direction drives the other, so an unguarded
//!   pair recurses until the stack goes. A test that only checked the state
//!   afterwards would pass on the version that overflows.
//! * **Jumping expands.** The column is how you reach a message you have
//!   already read, and those open collapsed — landing on a one-line header
//!   would make the index useless for the case it exists for.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

fn message(id: i64, seen: bool) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Norwood"), "ada@example.com")),
        subject: Some(format!("Tide gate interlock {id}")),
        preview: Some(format!("Snippet {id}")),
        received_at: chrono::Utc::now() - chrono::Duration::minutes(100 - id),
        seen,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 6,
        participants: Vec::new(),
    }
}

pub fn the_column_and_the_conversation_share_one_current_message() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    // The pane draws headers with no bodies until a factory is set, which is
    // what a window with nothing wired to it should do. This test is about
    // the two surfaces agreeing, not about what goes in the slots.
    let messages: Vec<Row> = (0..6).map(|id| message(id, id < 3)).collect();
    window.show_thread(ThreadId::new(1), Some("Tide gate interlock"), messages, 6);
    while gtk::glib::MainContext::default().iteration(false) {}

    let pane = window.conversation();
    let column = window.thread();

    // ── opening agrees, and agrees on the first unread ──────────────────
    assert_eq!(
        pane.focused(),
        Some(MessageId::new(3)),
        "the conversation opens on the first unread"
    );
    assert_eq!(
        column.cursor(),
        pane.focused(),
        "the index has to point at whatever the content opened on"
    );
    assert!(
        pane.widget().is_visible(),
        "opening a thread gives the reading pane to the conversation"
    );

    // ── moving the column moves the pane, and does not ring ─────────────
    // `prev_row` is `k`. If the two surfaces drove each other unguarded this
    // is where the stack would go, so reaching the assertion at all is half
    // of what is being tested.
    column.prev_row();
    while gtk::glib::MainContext::default().iteration(false) {}
    let after_k = column.cursor().expect("the column has a cursor");
    assert_eq!(
        pane.focused(),
        Some(after_k),
        "moving the index has to move the conversation"
    );
    assert!(
        pane.is_expanded(after_k),
        "jumping to a read message has to open it — the index exists to \
         reach messages you have already read, and those open collapsed"
    );

    // ── moving the pane moves the column ────────────────────────────────
    pane.focus_message(MessageId::new(5));
    while gtk::glib::MainContext::default().iteration(false) {}
    assert_eq!(pane.focused(), Some(MessageId::new(5)));
    assert_eq!(
        column.cursor(),
        Some(MessageId::new(5)),
        "the index has to follow the conversation too, or scrolling the pane \
         leaves the column pointing somewhere else"
    );

    // ── leaving closes the column, and only the column ──────────────────
    // #755: the pane is the conversation (ADR 0015 Q4) and the list cursor
    // is still on the row that opened it, so `Esc` must not swap a
    // single-message reader in on the way out. The pane leaves when the
    // cursor lands on a row that is not a conversation — `show_message`'s
    // job now, not this one's.
    window.close_thread();
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(
        pane.widget().is_visible(),
        "closing the index column must not take the conversation with it"
    );

    window.close();
}
