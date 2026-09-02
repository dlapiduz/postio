//! One owner for the reading pane (#502).
//!
//! Three surfaces live in `shell.reader()` — the reader, the search preview
//! and the composer — and each used to toggle its own visibility from its own
//! private snapshot: the window un-hid the reader whenever a message opened,
//! the search view restored whatever it had displaced at search entry, and
//! the composer showed *every* sibling on close. Three owners, one box, and
//! the screenshots to prove it: a message drawn twice (reader above, preview
//! below, each with its own remote-image banner), and a cleared preview
//! hanging under an inbox message after search was dismissed.
//!
//! The contract under test: **at most one occupant of the reading pane is
//! visible at a time**, whatever order search, reading and composing come
//! and go in — and what shows after a surface leaves is computed from what
//! is still active, never replayed from a snapshot.
//!
//! Skips without a display. One test function, for the reason `gtk_style.rs`
//! gives.

use crate::settle as pump;
use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::search::View;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{MailboxId, MessageId, ThreadId};
use postio_model::{Draft, EmailAddress, MessageBody};
use postio_search::SearchHit;

pub fn the_reading_pane_has_one_visible_occupant_at_a_time() {
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
    let preview = view.preview();

    // -- reading, then searching: the preview takes the pane --------------

    window.show_message(&body(), Some("ada@example.com"));
    pump();
    assert!(window.reader().widget().is_visible());
    assert!(!preview.is_visible());

    window.open_finder(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: "radon".to_owned(),
    });
    pump();
    view.set_focused(Some(&hit(7)));
    pump();
    assert!(preview.is_visible(), "searching, the preview is the pane");
    assert!(
        !window.reader().widget().is_visible(),
        "and the reader has stepped aside, not stacked underneath"
    );

    // -- opening a result: the reader takes the pane back ------------------
    //
    // `Enter` on a result sends `OpenMessage`, which lands here as
    // `show_message`. This is the double-drawn screenshot: the reader came
    // back on top of a preview that never left.

    window.show_message(&body(), Some("ada@example.com"));
    pump();
    assert!(
        window.reader().widget().is_visible(),
        "opening a result reads it in the real reader"
    );
    assert!(
        !preview.is_visible(),
        "one message must not be drawn twice — reader above, preview below"
    );

    // -- arrowing on through the results: back to previewing --------------

    view.set_focused(Some(&hit(8)));
    pump();
    assert!(
        preview.is_visible(),
        "moving the focus through results previews them again"
    );
    assert!(!window.reader().widget().is_visible());

    // -- dismissing search: what shows is what is still active -------------
    //
    // The pane was open on a message before search began, so the reader
    // comes back — and the preview leaves *entirely*. The second screenshot:
    // a cleared preview (kicker, "Arrow through the results…", `Open Ret`)
    // hanging under an inbox message after search was gone.

    window.close_finder();
    pump();
    assert!(
        !preview.is_visible(),
        "a dismissed search leaves nothing of its preview on screen"
    );
    assert!(
        window.reader().widget().is_visible(),
        "the message the pane was open on is what shows again"
    );

    // -- the composer outranks everything, and leaving it restores ---------

    window
        .composer()
        .open(Draft::new(postio_model::ids::AccountId::new(1)));
    pump();
    assert!(window.composer().is_visible());
    assert!(!window.reader().widget().is_visible());
    assert!(!preview.is_visible());

    window.composer().discard();
    pump();
    assert!(
        window.reader().widget().is_visible(),
        "closing the composer restores the occupant the state calls for \
         (occupant: {:?}, composer visible: {}, composer open: {})",
        window.shell().reader_occupant(),
        window.composer().is_visible(),
        window.composer().is_open(),
    );
    assert!(
        !preview.is_visible(),
        "— not every sibling it happened to hide on the way in"
    );

    // -- composing during a search falls back to the preview ---------------

    window.open_finder(Mode::Search);
    finder.set_query(Query {
        mode: Mode::Search,
        text: "radon".to_owned(),
    });
    pump();
    view.set_focused(Some(&hit(9)));
    pump();
    assert!(
        preview.is_visible(),
        "back in search, the preview has the pane again (occupant: {:?})",
        window.shell().reader_occupant()
    );
    window
        .composer()
        .open(Draft::new(postio_model::ids::AccountId::new(1)));
    pump();
    assert!(window.composer().is_visible());
    assert!(!preview.is_visible());

    window.composer().discard();
    pump();
    assert!(
        preview.is_visible(),
        "search is still the active surface, so its preview returns \
         (occupant now: {:?})",
        window.shell().reader_occupant()
    );
    assert!(!window.reader().widget().is_visible());

    window.destroy();
}

fn body() -> MessageBody {
    MessageBody {
        text: Some("The radon report, for review.".to_string()),
        html: None,
    }
}

fn hit(id: i64) -> SearchHit {
    SearchHit {
        message_id: MessageId::new(id),
        thread_id: Some(ThreadId::new(id)),
        mailbox_id: MailboxId::new(1),
        subject: Some(format!("Result {id}")),
        from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
        received_at: chrono::Utc::now(),
        snippet: "the radon report".to_string(),
        score: -1.0,
    }
}
