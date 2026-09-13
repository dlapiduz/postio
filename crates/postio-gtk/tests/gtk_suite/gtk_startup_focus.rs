//! #1473: a freshly presented window puts the keyboard on the first message.
//!
//! `Window::present` has always *intended* this -- it grabs focus on the list
//! before presenting. What it could not do is hold it. At the moment the
//! composition root presents, the list is still empty: `set_source` and the
//! first `deliver` come afterwards, when the store answers. So the grab lands
//! on a `GtkListView` with no rows in it, and the moment rows arrive GTK
//! re-runs its focus traversal and hands the keyboard to the first focusable
//! widget in the tree instead -- the header's search field.
//!
//! That is why the order here is present-then-fill rather than the other way
//! round: filled first, the grab sticks and nothing fails. The bug lives
//! entirely in the gap between the two, which is the gap the real app has.
//!
//! Asserted through the window's focus widget rather than the list's own idea
//! of its cursor: a list that believes it holds the cursor while GTK routes
//! keys to the header is exactly the state that shipped, so only the
//! toolkit's answer settles it.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::{GtkWindowExt, WidgetExt};
use postio_gtk::list::{PageSource, Row};
use postio_gtk::row::MessageRowView;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::MessageId;

const ROWS: u32 = 6;

struct Pages;

impl PageSource for Pages {
    fn total(&self) -> u32 {
        ROWS
    }
    fn request(&self, _page: u32) {}
}

fn row(position: u32) -> Row {
    Row {
        id: MessageId::new(position as i64 + 1),
        thread: None,
        from: Some(postio_model::address::EmailAddress::new(
            Some("Ada Lovelace"),
            "ada@example.com",
        )),
        subject: Some(format!("Note {position}")),
        preview: Some("…".into()),
        received_at: Utc.with_ymd_and_hms(2026, 8, 23, 9, 0, 0).unwrap(),
        seen: true,
        flagged: false,
        answered: false,
        send_state: None,
        send_at: None,
        has_attachments: false,
        thread_count: 1,
        participants: Vec::new(),
    }
}

/// Pumps idle and frame-clock sources until `done`, as `gtk_composer_focus`
/// does -- a real regression still fails when the deadline runs out, a slow
/// machine only takes longer to say so.
fn settle_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_millis(3000));
    while !done() && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

/// Which message the focused row is showing, if the focus is on a row at all.
///
/// Lifted from `gtk_list_focus_return.rs`, and for the reason its own comment
/// gives: the focusable widget is each row's `GtkListItem` parent, not the
/// `MessageRowView`, so "which row has the keyboard" can only be answered by
/// walking them.
fn cursor_message(window: &Window) -> Option<MessageId> {
    let focused = GtkWindowExt::focus(window)?;
    let found: RefCell<Option<MessageId>> = RefCell::new(None);
    window.list().each_row(|view: &MessageRowView| {
        if view.parent().as_ref() == Some(&focused) {
            *found.borrow_mut() = view.row().map(|r| r.id);
        }
    });
    found.into_inner()
}

pub fn a_presented_window_puts_the_keyboard_on_the_first_message() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();

    // The composition root's order: present the window, then fill it once the
    // store answers. Reversing these two lines is what makes this pass on
    // unfixed code.
    window.present();
    window.list().model().set_source(Rc::new(Pages));
    window
        .list()
        .model()
        .deliver(0, (0..ROWS).map(row).collect());

    let first = MessageId::new(1);
    settle_until(|| cursor_message(&window) == Some(first));

    assert_eq!(
        cursor_message(&window),
        Some(first),
        "the keyboard belongs on the first message once mail arrives -- a key \
         pressed straight after launch has to act on that message rather than \
         type into the header. Focus was on {:?}",
        GtkWindowExt::focus(&window).map(|w| w.widget_name())
    );

    // The finder is the widget that was taking it, and the symptom reported
    // from the live app was its box sitting open from launch.
    assert!(
        !window.finder().is_open(),
        "nothing has asked for the finder yet, so it stays shut"
    );
}
