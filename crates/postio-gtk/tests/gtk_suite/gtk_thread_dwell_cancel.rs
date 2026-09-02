//! Opening a conversation stops the list's clock (#797).
//!
//! Two dwell timers exist, and they measure different things. The list's
//! (#71) says "this row was in front of a person long enough to have been
//! read", and a thread row's id is its *representative* — the newest message
//! in that folder. The conversation pane's says the same thing about the
//! message focus is resting on, which is what ADR 0015 Q4 actually asks for:
//! reading is per message, driven by focus, and never "opened the thread, all
//! six read".
//!
//! So once the conversation is what the reader is looking at, the list's
//! clock is measuring a row nobody is looking at any more, and its message is
//! one focus may never reach. It has to stop — the same call the composer
//! taking the pane and the window going inactive already make, for the reason
//! they already give.
//!
//! # Why this is a test and not a comment
//!
//! It used to stop by accident: `sync_reading_pane` cancels the dwell when
//! `!self.reading()`, and whether that ran before the timer fired was a race.
//! It won on a developer's machine and lost on a CI runner, where
//! `app_suite/thread_dwell.rs` failed with "opening a conversation must not
//! read messages focus never reached". A race that usually wins is the worst
//! kind of correct, so this asserts the cancellation directly rather than
//! waiting to see whether a read shows up.
//!
//! The delay is shortened with `set_dwell_delay`, so the wait below is
//! bounded and the test does not depend on losing a race the other way.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use postio_gtk::list::{PageSource, Row};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

const ROWS: u32 = 6;

struct Pages;

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, _page: u32) {}
}

/// Short enough to keep the test quick, long enough that the pump below is
/// not racing the timer it is trying to prove was cancelled.
const DWELL: Duration = Duration::from_millis(40);

fn message(id: u32, unread: bool) -> Row {
    let id = id as i64 + 1;
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Norwood"), "ada@example.com")),
        subject: Some(format!("Tide gate interlock {id}")),
        preview: Some(format!("Snippet {id}")),
        received_at: chrono::Utc::now() - chrono::Duration::minutes(100 - id),
        seen: !unread,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 6,
        participants: Vec::new(),
    }
}

pub fn opening_a_conversation_stops_the_lists_clock() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    let list = window.list();
    list.set_dwell_delay(DWELL);

    let dwelled: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    list.connect_dwelled({
        let dwelled = dwelled.clone();
        move |message| dwelled.borrow_mut().push(message)
    });

    // Arm the clock the way a person does: land the cursor on a row. The
    // autoselect parks on row 0 and `report_cursor` dedupes, so `next_row`
    // is the first real landing -- row 1, id 2. That row stands for a
    // conversation, and its id is the representative.
    list.model().set_source(Rc::new(Pages));
    list.model()
        .deliver(0, (0..ROWS).map(|id| message(id, id >= 3)).collect());
    while gtk::glib::MainContext::default().iteration(false) {}
    list.next_row();
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(
        dwelled.borrow().is_empty(),
        "the clock should be running, not already fired"
    );

    // ── the conversation takes over ──────────────────────────────────────
    let messages: Vec<Row> = (0..6).map(|id| message(id, id >= 3)).collect();
    window.show_thread(ThreadId::new(1), Some("Tide gate interlock"), messages, 6);

    // Well past the delay, pumping throughout: if the clock were still
    // running it has had every chance to fire.
    let deadline = std::time::Instant::now() + DWELL * 10;
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(
        dwelled.borrow().is_empty(),
        "opening a conversation left the list's dwell running, so {:?} was \
         marked read without focus ever reaching it — 'opened the thread, all \
         six read' is what ADR 0015 Q4 forbids, and the conversation pane's \
         own per-message dwell is what should decide this",
        dwelled.borrow()
    );

    // ── and the same for the other way in ────────────────────────────────
    //
    // There are two routes to a conversation: the drill-in (`show_thread`,
    // above) and the list opening one directly, which #755 added. Both pass
    // through `show_conversation`, and that is where the cancellation lives
    // precisely so a third route cannot arrive without it. Asserted against
    // the choke point rather than against either caller: pinning it to one
    // caller is how the first attempt at this fix passed here and still let
    // `app_suite/thread_dwell.rs` fail on CI, which reaches the pane by the
    // route the fix did not cover.
    list.next_row();
    while gtk::glib::MainContext::default().iteration(false) {}
    window.show_conversation((0..6).map(|id| message(id, id >= 3)).collect());

    let deadline = std::time::Instant::now() + DWELL * 10;
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        dwelled.borrow().is_empty(),
        "raising the conversation pane directly left the list's clock \
         running: {:?}",
        dwelled.borrow()
    );
}
