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

/// `/` opens the box and `Escape` puts it away again.
///
/// Reported from a live run alongside the blank plate: `/` focused the field
/// and `Escape` did not get back out. Both halves are asserted here because
/// the pair is the whole gesture -- a box that opens and will not close is
/// worse than one that never opened, since every subsequent single key is
/// typed into it rather than acting on the mail.
///
/// Driven through `handle_key`, which is the seam a real press arrives at,
/// rather than by calling `open` and `press_escape` directly: what is in
/// doubt is whether the key reaches the command at all while the keyboard is
/// inside the entry, and calling the methods would assume the answer.
pub fn slash_opens_the_box_and_escape_puts_it_away() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    settle_until(|| false_once());

    window.handle_key(
        gdk::Key::from_name("slash").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle_until(|| window.finder().is_open());
    assert!(
        window.finder().is_open(),
        "`/` opens the box -- it is the one key the whole search gesture starts with"
    );

    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle_until(|| !window.finder().is_open());
    assert!(
        !window.finder().is_open(),
        "`Escape` means get me out of here, and a box that will not close \
         swallows every key pressed after it"
    );
}

/// Pumps a few frames without waiting for anything in particular.
fn false_once() -> bool {
    use std::cell::Cell;
    thread_local! { static SEEN: Cell<u8> = const { Cell::new(0) }; }
    SEEN.with(|seen| {
        let n = seen.get().saturating_add(1);
        seen.set(n);
        n > 3
    })
}

/// `Escape` out of the box puts the keyboard back on the row it left.
///
/// The maintainer's own words: Escape "should not only close the search but
/// focus back to the message list wherever it was before starting
/// searching". Closing the box already restored the *pane* -- `close_finder`
/// remembers which one was focused -- but its own comment concedes that
/// memory "has no shape to record which row on it had the keyboard", so the
/// keyboard came back to the `GtkListView` itself and the next key had no row
/// to act on. Identical in shape to the launch bug above, and fixed with the
/// same call.
///
/// The cursor is put on a middle row first, so a fix that merely went back to
/// the top would still fail this.
pub fn escape_out_of_the_box_returns_the_keyboard_to_the_row_it_left() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    window.list().model().set_source(Rc::new(Pages));
    window
        .list()
        .model()
        .deliver(0, (0..ROWS).map(row).collect());
    settle_until(|| cursor_message(&window).is_some());

    // Down to the third row, by the key a person would use.
    for _ in 0..2 {
        window.handle_key(
            gdk::Key::from_name("j").unwrap(),
            gdk::ModifierType::empty(),
        );
    }
    settle_until(|| cursor_message(&window) == Some(MessageId::new(3)));
    let left_from = cursor_message(&window).expect("the cursor is on a row");
    assert_eq!(left_from, MessageId::new(3), "j moved the cursor twice");

    window.handle_key(
        gdk::Key::from_name("slash").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle_until(|| window.finder().is_open());
    assert!(window.finder().is_open(), "`/` opens the box");

    window.handle_key(
        gdk::Key::from_name("Escape").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle_until(|| cursor_message(&window) == Some(left_from));

    assert!(!window.finder().is_open(), "`Escape` closes the box");
    assert_eq!(
        cursor_message(&window),
        Some(left_from),
        "the keyboard comes back to the row it left, not to the pane that \
         holds it -- focus was on {:?}",
        GtkWindowExt::focus(&window).map(|w| w.widget_name())
    );
}
