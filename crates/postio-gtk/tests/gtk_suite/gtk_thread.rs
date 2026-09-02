//! Thread drill-in on a real display: `t` in, `Esc` out, and nothing lost in
//! between.
//!
//! The ordering, the filtering and the summary line are pure and unit-tested
//! in `thread.rs`. What needs a display is the round trip: that `t` puts the
//! thread where the list was, that `j`/`k` then move the *thread's* cursor
//! rather than the list's, that `Esc` brings the list back with its scroll
//! position, cursor and selection exactly as they were, and that none of it
//! costs more than a frame.
//!
//! The wall-clock budget is *not* here. `benches/thread_drill.rs` measures it,
//! which is where CLAUDE.md puts performance budgets and the only place a
//! developer machine running four other builds cannot turn one into a flake.
//!
//! Skips without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Context};
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::thread::Order;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// Big enough that a drill-in over it is a real measurement — the bead asks
/// for a 200-message thread specifically.
const THREAD_SIZE: i64 = 200;

pub fn t_drills_into_a_thread_and_esc_puts_the_list_back_exactly() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let account = AccountId::new(1);
    let source = Rc::new(Mailbox200::new());
    let feeds = window.install_feeds(account, "lena@example.com", source.clone(), source);
    window.present();
    pump();
    settle(&window);

    // Whatever the window told the application. The drill-in has to reach it
    // — `AppState` keeps the back stack, and a view change it never hears
    // about is one it cannot undo.
    let told: Rc<RefCell<Vec<postio_core::Command>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_action({
        let told = told.clone();
        move |command| told.borrow_mut().push(command)
    });

    // -- a position in the list worth not losing --------------------------

    let list = window.list();
    list.first_row();
    // Far enough down that the list has genuinely scrolled: a cursor still in
    // the first viewport would leave nothing to prove about the scroll
    // position on the way back.
    for _ in 0..60 {
        list.next_row();
    }
    settle(&window);
    list.toggle_cursor_row();
    list.extend_down();
    pump();

    let cursor_before = list.cursor_id().expect("a cursor row");
    let selection_before = list.selection().selection();
    let scroll_before = scroll_offset(&window);
    assert!(!list.selection().is_empty(), "a selection worth restoring");
    assert!(
        scroll_before > 0.0,
        "and a scroll position worth restoring, not the top of the list"
    );

    // -- t turns the column into the thread -------------------------------

    assert!(!window.thread_open());
    window.act(postio_core::Command::Thread { thread: None });
    pump();

    assert!(window.thread_open(), "the column is the thread now");
    // -- j and k move the thread's cursor, not the list's ------------------

    let first = window.thread().cursor();
    window.act(postio_core::Command::default_for(CommandId::PrevMessage));
    pump();
    assert_ne!(window.thread().cursor(), first, "`k` moved in the thread");
    assert_eq!(
        list.cursor_id(),
        Some(cursor_before),
        "and left the list's own cursor alone"
    );

    // -- the view options work while drilled in ---------------------------
    //
    // Through `window.act`, the same path the keyboard and the palette
    // resolve a keystroke to (`Command::default_for` -> `act`) -- not
    // `ThreadView::set_order`/`set_unread_only` directly, which would only
    // prove the widget works and say nothing about whether `postio-yzc`'s
    // two new commands actually reach it.

    assert_eq!(window.thread().order(), Order::Oldest, "the starting order");
    window.act(postio_core::Command::default_for(
        CommandId::ToggleThreadOrder,
    ));
    pump();
    assert_eq!(window.thread().order(), Order::Newest);
    let newest = window.thread().rows();
    assert_eq!(
        newest.first().map(|row| row.id),
        Some(MessageId::new(THREAD_SIZE)),
        "reversed"
    );

    assert!(!window.thread().unread_only(), "the starting filter");
    window.act(postio_core::Command::default_for(
        CommandId::ToggleThreadUnread,
    ));
    pump();
    assert!(window.thread().unread_only());
    assert!(
        window.thread().rows().iter().all(|row| !row.seen),
        "the filter keeps only what has not been read"
    );
    assert!(
        !window.thread().rows().is_empty(),
        "and the fixture has unread mail in it, or this proves nothing"
    );

    // -- and the footer draws the keys that reach them ---------------------
    let hints = key_hints(&window);
    assert!(
        hints.iter().any(|hint| hint == "n"),
        "the unread toggle should show the key that reaches it: {hints:?}"
    );
    assert!(
        hints.iter().any(|hint| hint == "o"),
        "the order toggle should show the key that reaches it: {hints:?}"
    );

    // -- Esc puts the list back, exactly ----------------------------------

    window.act(postio_core::Command::Back);
    pump();
    // Wait for the scroller to be back where it was, rather than for ten
    // frames and a hope. See `settle_until` — under load the allocation is
    // not finished when the frames run out, and the offset read then is
    // stale rather than wrong.
    settle_until(&window, || scroll_offset(&window) == scroll_before);

    assert!(!window.thread_open());
    assert!(list_is_showing(&window));
    assert_eq!(window.context(), Context::List);
    assert_eq!(
        list.cursor_id(),
        Some(cursor_before),
        "the cursor is where it was"
    );
    assert_eq!(
        list.selection().selection(),
        selection_before,
        "and so is the selection"
    );
    assert_eq!(
        scroll_offset(&window),
        scroll_before,
        "and the scroll position, to the pixel — canvas 3a promises the round \
         trip puts you back exactly, and `settle_until` has already waited \
         for the allocation to finish, so this is a real loss of position \
         rather than a frame that had not landed yet"
    );

    // -- and the column forgets the thread it was showing ------------------

    assert!(window.thread().thread().is_none());
    assert!(window.thread().rows().is_empty());
    assert_eq!(
        window.thread().order(),
        Order::Oldest,
        "a fresh drill-in starts from the conversation's own order"
    );
    assert!(!window.thread().unread_only());

    // -- Esc out of a thread never falls through to the selection ----------

    window.act(postio_core::Command::Thread { thread: None });
    pump();
    window.act(postio_core::Command::Back);
    pump();
    assert_eq!(
        list.selection().selection(),
        selection_before,
        "`Esc` meant `leave the thread`, which is nearer than the selection"
    );

    drop(feeds);
    window.destroy();
}

/// A mailbox whose rows are all one thread, so the drill-in has something the
/// size the bead asks about.
struct Mailbox200 {
    rows: Vec<Row>,
}

impl Mailbox200 {
    fn new() -> Self {
        let base = chrono::Utc::now() - chrono::Duration::days(30);
        let rows = (1..=THREAD_SIZE)
            .map(|index| Row {
                id: MessageId::new(index),
                thread: Some(ThreadId::new(1)),
                from: Some(EmailAddress::new(
                    Some(format!("Correspondent {}", index % 4)),
                    format!("person{}@example.org", index % 4),
                )),
                subject: Some(format!("Re: index rebuild ({index})")),
                preview: Some("…".to_owned()),
                // Newest last, so the list's own reverse-chronological order
                // and the thread's oldest-first order really do differ.
                received_at: base + chrono::Duration::hours(index),
                seen: index % 5 != 0,
                flagged: false,
                answered: false,
                draft: false,
                has_attachments: false,
                thread_count: THREAD_SIZE as u32,
                participants: Vec::new(),
            })
            .rev()
            .collect();
        Mailbox200 { rows }
    }
}

impl MessageSource for Mailbox200 {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = self.rows.len() as u32;
        let start = (request.offset as usize).min(self.rows.len());
        let end = (start + request.limit as usize).min(self.rows.len());
        let rows = self.rows[start..end].to_vec();
        Box::pin(async move { Ok(Page { total, rows }) })
    }
}

impl MailboxSource for Mailbox200 {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(1);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: THREAD_SIZE as u32,
            unread: 40,
            flagged: 0,
            snoozed: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

/// Whether the message list — and the named states over it — are on screen.
fn list_is_showing(window: &Window) -> bool {
    find(&window.clone().upcast(), &|widget| {
        widget.type_().name() == "PostioMessageListView"
    })
    .and_then(|list| list.parent())
    .is_some_and(|overlay| overlay.property::<bool>("visible"))
}

/// Where the list is scrolled to, in pixels.
fn scroll_offset(window: &Window) -> f64 {
    find(&window.clone().upcast(), &|widget| {
        widget.type_().name() == "PostioMessageListView"
    })
    .and_then(|list| find(&list, &|widget| widget.is::<gtk::ScrolledWindow>()))
    .and_then(|scroller| scroller.downcast::<gtk::ScrolledWindow>().ok())
    .map(|scroller| scroller.vadjustment().value())
    .unwrap_or(0.0)
}

/// The text of every key hint currently drawn in the thread column.
///
/// `postio-yzc`: the unread and order toggles used to be bare buttons with no
/// hint at all, because neither had a command to be a hint *for*. Reading the
/// hints back out of the widget tree, rather than the toggle buttons'
/// registry-blind old text, is what tells a regression that removed
/// `postio_gtk::header::labelled` from them apart from one that only changed
/// their wording.
fn key_hints(window: &Window) -> Vec<String> {
    let mut hints = Vec::new();
    walk(&window.clone().upcast(), &mut |widget| {
        if widget.has_css_class("postio-keyhint")
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
        {
            hints.push(label.text().to_string());
        }
    });
    hints
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(node) = child {
        walk(&node, visit);
        child = node.next_sibling();
    }
}

/// Let the frame clock tick, so the list actually asks for and receives its
/// pages — the rows arrive a frame or two after the viewport exists.
fn settle(window: &Window) {
    let left = Rc::new(std::cell::Cell::new(10u32));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    });
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_millis(3000);
    while left.get() > 0 && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

/// Drive the main loop until `done`, or give up after a deadline.
///
/// [`settle`] waits a fixed ten frame ticks, which is a bet that ten frames is
/// long enough for whatever is being waited on. On an idle box it is; with
/// four sessions building it is not, and a scrolled window that has not
/// finished its allocation reports an offset that is merely stale rather than
/// wrong (`postio-1ff`).
///
/// So: wait for the condition rather than for a number of frames. A genuine
/// regression still fails — the deadline runs out and the assertion after this
/// call reports the real values — but a slow machine only makes it slower,
/// which is the difference between a test that is strict and one that is
/// flaky.
fn settle_until(window: &Window, done: impl Fn() -> bool) {
    settle(window);
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + Duration::from_millis(3000);
    while !done() && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
