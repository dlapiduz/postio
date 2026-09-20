//! A batch landing mid-sync must not move what the user is holding.
//!
//! `postio-qhz.7`'s fourth acceptance criterion. A sync pass announces itself
//! as its batches commit, and each announcement costs the list a reload — so
//! the reload is not a rare event during a first sync, it is the steady state
//! for minutes. If a reload moved the cursor or dropped the marked messages,
//! `x x x a` would archive the wrong mail, and it would do it only on a real
//! account with a real sync running, which is the hardest place to notice.
//!
//! Two separate things have to survive, and they survive for different
//! reasons. The *selection* is `SelectionState`, a set of `MessageId`s, so it
//! is not addressed by position at all. The *cursor* is a `GtkSingleSelection`
//! over the model, which is addressed by position — it survives because
//! `MessageList::invalidate` re-emits `items_changed` over the same length
//! rather than resetting the model, and because the page that comes back
//! carries the same ids in the same order.
//!
//! Skips without a display. Nothing here touches the network.

use crate::pump;
use std::cell::Cell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::Event;
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const ACCOUNT: i64 = 1;
const INBOX: i64 = 1;

/// A mailbox a sync is still filling.
///
/// Rows are oldest-last and their ids are stable, which is what an initial
/// sync actually does: it walks the newest UID down, so a batch lands at the
/// *end* of the list and the rows already on screen keep their positions.
struct Filling {
    total: Cell<u32>,
}

impl Filling {
    fn new(total: u32) -> Rc<Self> {
        Rc::new(Filling {
            total: Cell::new(total),
        })
    }

    /// A batch committed: the folder is longer than it was.
    fn batch_committed(&self, rows: u32) {
        self.total.set(self.total.get() + rows);
    }
}

impl MailboxSource for Filling {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let account = AccountId::new(ACCOUNT);
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: self.total.get(),
            unread: self.total.get(),
            flagged: 0,
            snoozed: 0,
            attention: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

impl MessageSource for Filling {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = self.total.get();
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| Row {
                    id: MessageId::new(position as i64 + 1),
                    thread: None,
                    from: None,
                    subject: Some(format!("message {position}")),
                    preview: None,
                    received_at: Utc
                        .timestamp_opt(1_700_000_000 - position as i64, 0)
                        .unwrap(),
                    seen: false,
                    flagged: false,
                    answered: false,
                    send_state: None,
                    send_at: None,
                    has_attachments: false,
                    thread_count: 1,
                    participants: Vec::new(),
                })
                .collect();
            Ok(Page { total, rows })
        })
    }
}

pub fn a_batch_arriving_mid_sync_leaves_the_cursor_and_the_selection_alone() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Filling::new(120);
    let window = Window::default();
    window.present();
    pump();

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let list = window.list();
    pump_until(|| list.model().n_items() == 120);
    assert_eq!(list.model().n_items(), 120, "the folder as it stands");

    // Where the user is: a few rows down, with three messages marked.
    for _ in 0..6 {
        list.next_row();
    }
    pump();
    let cursor = list.cursor_id().expect("the cursor is on a row");
    let marked = [MessageId::new(2), MessageId::new(4), MessageId::new(9)];
    list.selection().extend_over(marked);
    let selection = list.selection().selection();

    // A batch commits. The engine coalesces these and announces the mailbox
    // moved; it does not say how, because it does not know.
    store.batch_committed(200);
    feeds.apply(&Event::MessageListChanged {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
    });
    pump_until(|| list.model().n_items() == 320);

    assert_eq!(
        list.model().n_items(),
        320,
        "the batch has to actually arrive, or this test proves nothing"
    );
    assert_eq!(
        list.cursor_id(),
        Some(cursor),
        "the cursor moved to a different message when a batch landed"
    );
    assert_eq!(
        list.selection().selection(),
        selection,
        "marked messages were lost when a batch landed; the next verb would \
         have hit the wrong mail"
    );
    for message in marked {
        assert!(
            list.selection().contains(message),
            "message {} is no longer marked",
            message.get()
        );
    }

    // And again, several times over, because a real sync does this for
    // minutes rather than once.
    for batch in 0..5 {
        store.batch_committed(200);
        feeds.apply(&Event::MessageListChanged {
            account: postio_model::AccountId::new(1),
            mailbox: MailboxId::new(INBOX),
        });
        let expected = 520 + batch * 200;
        pump_until(|| list.model().n_items() == expected);
    }
    assert_eq!(list.model().n_items(), 1_320);
    assert_eq!(list.cursor_id(), Some(cursor), "drifted over six batches");
    assert_eq!(list.selection().selection(), selection);
}

/// Pump until `done`, or give up after a deadline.
///
/// `pump` spins two hundred times over whatever is *currently* pending, which
/// is a bet that the work will have been queued by the time it looks. With
/// several sessions building, a task can be spawned and not yet scheduled
/// when `pending()` reports false, so `pump` returns before the page it set
/// in motion has landed and the count read after it is one batch stale
/// (`postio-mdu1`).
///
/// Waiting on the condition keeps the assertion exactly as strict — a batch
/// that genuinely never arrives still fails, after the deadline — while a
/// loaded machine only makes it wait longer.
fn pump_until(done: impl Fn() -> bool) {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while std::time::Instant::now() < deadline {
        pump();
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A mailbox whose *order* moves, which is what `invalidate` is for.
///
/// `Filling` above only ever appends, so every row keeps its position and a
/// cursor addressed by position lands on the same message by luck. That is
/// the assumption this file's own module doc names — "because the page that
/// comes back carries the same ids in the same order" — and #1177 is what
/// happens when it does not hold.
struct Reordering {
    order: std::cell::RefCell<Vec<i64>>,
}

impl Reordering {
    fn new(count: i64) -> Rc<Self> {
        Rc::new(Reordering {
            order: std::cell::RefCell::new((1..=count).collect()),
        })
    }

    /// The newest message is now the oldest, and everything shifted by one.
    ///
    /// A rotation rather than a shuffle because it moves every row without
    /// changing the length, so nothing else about the list can explain a
    /// cursor that ends up somewhere new.
    fn rotate(&self) {
        self.order.borrow_mut().rotate_left(1);
    }

    /// The oldest message is now the newest, and everything shifted down by
    /// one.
    ///
    /// The same move in the other direction, which is what a sync that finds
    /// a message older than the page it already had looks like: every row
    /// moves, and the one that was on top is still on the first page.
    fn rotate_back(&self) {
        self.order.borrow_mut().rotate_right(1);
    }
}

impl MailboxSource for Reordering {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let account = AccountId::new(ACCOUNT);
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(INBOX);
        inbox.role = MailboxRole::Inbox;
        let total = self.order.borrow().len() as u32;
        inbox.counts = MailboxCounts {
            total,
            unread: total,
            flagged: 0,
            snoozed: 0,
            attention: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

impl MessageSource for Reordering {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let order = self.order.borrow().clone();
        Box::pin(async move {
            let total = order.len() as u32;
            let end = ((request.offset + request.limit) as usize).min(order.len());
            let rows = order[request.offset as usize..end]
                .iter()
                .enumerate()
                .map(|(index, id)| Row {
                    id: MessageId::new(*id),
                    thread: None,
                    from: None,
                    subject: Some(format!("message {id}")),
                    preview: None,
                    received_at: Utc.timestamp_opt(1_700_000_000 - index as i64, 0).unwrap(),
                    seen: false,
                    flagged: false,
                    answered: false,
                    send_state: None,
                    send_at: None,
                    has_attachments: false,
                    thread_count: 1,
                    participants: Vec::new(),
                })
                .collect();
            Ok(Page { total, rows })
        })
    }
}

/// The cursor is on a *message*, not on a row number (#1177).
///
/// `MessageListChanged` means the order moved — that is what the event is for
/// — and `MessageList::invalidate` answers it by telling GTK every row was
/// removed and re-added. `GtkSingleSelection` keeps its *position* across
/// that, so when the rows underneath have moved, the cursor is left pointing
/// at whichever message now happens to be at that index and the reading pane
/// follows it there.
///
/// On launch that is the reported symptom: the first screen shows one message
/// and then jumps to another one nobody chose.
pub fn a_reordering_sync_leaves_the_cursor_on_the_same_message() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Reordering::new(120);
    let window = Window::default();
    window.present();
    pump();

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let list = window.list();
    pump_until(|| list.model().n_items() == 120);

    // Where the user is: a few rows down, reading something.
    for _ in 0..6 {
        list.next_row();
    }
    pump();
    let reading = list.cursor_id().expect("the cursor is on a row");

    // A sync pass reorders the folder and says so the only way it can.
    store.rotate();
    feeds.apply(&Event::MessageListChanged {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
    });
    pump_until(|| list.cursor_id() != Some(reading));

    assert_eq!(
        list.cursor_id(),
        Some(reading),
        "the order moved and the cursor stayed on the row number, so the \
         reading pane is now showing message {:?} instead of the one it was \
         opened on",
        list.cursor_id()
    );
}

/// A cursor nobody placed is still on a *message* (#1177).
///
/// This is the half of #1177 the first fix left open, and it is the half the
/// issue actually reports. At launch nobody has chosen anything:
/// `SingleSelection` autoselects row 0, the reading pane draws whatever that
/// points at, and then the first sync pass reorders the folder underneath it.
/// A pin that only holds for a cursor a person placed does nothing here, so
/// the first screen shows one message and jumps to another.
///
/// The rotation goes the other way from the test above so the pinned message
/// stays on page 0: `select_message` asks for the first page and gives up if
/// what it wants is not there, which is a separate behaviour with its own
/// reasons — this test is about the pin, not about paging.
pub fn a_reordering_sync_leaves_the_autoselected_cursor_on_its_message() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Reordering::new(120);
    let window = Window::default();
    window.present();
    pump();

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let list = window.list();
    pump_until(|| list.model().n_items() == 120);

    // Nobody has touched anything: this is a window that has just opened.
    let showing = list
        .cursor_id()
        .expect("the autoselect puts the cursor on the first row");
    assert_eq!(showing, MessageId::new(1), "the newest message is on top");
    assert!(!list.landed(), "nobody has chosen a row yet");

    // A sync pass finds an older message and says the only thing it can say.
    store.rotate_back();
    feeds.apply(&Event::MessageListChanged {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
    });
    pump_until(|| list.model().peek(0) == Some(MessageId::new(120)));
    assert_eq!(
        list.model().peek(0),
        Some(MessageId::new(120)),
        "the reordered page never arrived, so this test proves nothing"
    );
    pump();

    assert_eq!(
        list.cursor_id(),
        Some(showing),
        "the order moved and the cursor stayed on the row number, so the \
         first screen showed message {showing:?} and then jumped to {:?} — \
         which is what a person sees at launch, having chosen nothing",
        list.cursor_id()
    );
}

/// Putting the cursor back is not a person choosing a row (#71, #601).
///
/// The pin above reaches the cursor through the same `move_cursor_to` a
/// keystroke does, and that is what sets `landed` — the flag meaning "somebody
/// chose this row", which is what #71's dwell-to-read waits for. A pin that
/// set it would hand the launch case a cursor the application believes a
/// person placed, and the next thing the list reported would start the clock
/// on mail nobody has looked at: the unread signal destroying itself.
///
/// Proven where it shows rather than on the flag: the pinned message is moved
/// off the first page, so `select_message` gives up, the cursor reports the
/// row it is left on, and a dwell armed for that report would mark a message
/// read at launch.
pub fn restoring_the_cursor_after_a_reload_does_not_arm_the_dwell() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Reordering::new(120);
    let window = Window::default();
    window.present();
    pump();

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    let list = window.list();
    pump_until(|| list.model().n_items() == 120);
    assert!(!list.landed(), "nobody has chosen a row yet");

    let dwelled: Rc<std::cell::RefCell<Vec<MessageId>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));
    let read = Rc::clone(&dwelled);
    list.connect_dwelled(move |message| read.borrow_mut().push(message));
    // Short enough that a dwell which was armed has fired well inside the
    // settle below — the assertion is that nothing was armed, so the timer
    // must not be the thing that outlasts the wait.
    list.set_dwell_delay(std::time::Duration::from_millis(30));

    // A sync pass reorders the folder, and the cursor is pinned back onto the
    // message it was showing.
    store.rotate_back();
    feeds.apply(&Event::MessageListChanged {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
    });
    pump_until(|| list.model().peek(0) == Some(MessageId::new(120)));

    // A second pass carries it off the first page, which is as far as
    // `select_message` looks. It gives up, and the cursor reports whatever
    // row it was left on — the report a dwell would be armed for.
    for _ in 0..60 {
        store.rotate();
    }
    feeds.apply(&Event::MessageListChanged {
        account: postio_model::AccountId::new(1),
        mailbox: MailboxId::new(INBOX),
    });
    pump_until(|| list.cursor_id() != Some(MessageId::new(1)));
    settle_for(std::time::Duration::from_millis(300));

    assert!(
        dwelled.borrow().is_empty(),
        "launching and letting a sync run marked {:?} read, and nobody had \
         chosen a row: restoring the cursor claimed the landing as a \
         person's",
        dwelled.borrow()
    );
    assert!(
        !list.landed(),
        "a reload made the cursor a person's, so every surface that asks \
         whether somebody chose this row (#755) now gets the wrong answer"
    );
}

/// Pump the main loop for `how_long`, so a timer that was armed can fire.
///
/// The shape a *negative* needs: everywhere else here waits on a condition,
/// which cannot prove that nothing happens.
fn settle_for(how_long: std::time::Duration) {
    let deadline = std::time::Instant::now() + postio_test_support::scaled(how_long);
    while std::time::Instant::now() < deadline {
        pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A different question is not a reload (#1177).
///
/// `set_source` — another folder, a search — announces itself the same way
/// `invalidate` does, as a full replace over the same length, so the pin
/// cannot tell the two apart by shape. It tells them apart by *generation*: a
/// cursor means a message inside the answer to one question, and a folder's
/// cursor that happens to also be a hit is not where a search should open.
///
/// `app_suite/search_results.rs` is where this was found — the result list
/// opened on the message the mailbox had been showing instead of on the best
/// match — and this is the same rule one layer down, where it costs a second
/// rather than a minute.
pub fn switching_what_the_list_shows_does_not_carry_the_cursor_over() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    /// Six rows, in whatever order the question answers.
    struct Answer(Vec<i64>);

    impl postio_gtk::list::PageSource for Answer {
        fn total(&self) -> u32 {
            self.0.len() as u32
        }
        fn request(&self, _page: u32) {}
    }

    fn rows(ids: &[i64]) -> Vec<Row> {
        ids.iter()
            .enumerate()
            .map(|(index, id)| Row {
                id: MessageId::new(*id),
                thread: None,
                from: None,
                subject: Some(format!("message {id}")),
                preview: None,
                received_at: Utc.timestamp_opt(1_700_000_000 - index as i64, 0).unwrap(),
                seen: false,
                flagged: false,
                answered: false,
                send_state: None,
                send_at: None,
                has_attachments: false,
                thread_count: 1,
                participants: Vec::new(),
            })
            .collect()
    }

    let pane = postio_gtk::list_view::MessageListView::new();
    let window = gtk::Window::new();
    window.set_default_size(404, 600);
    window.set_child(Some(&pane));
    window.present();
    pump();

    let folder = [1, 2, 3, 4, 5, 6];
    pane.model().set_source(Rc::new(Answer(folder.to_vec())));
    pane.model().deliver(0, rows(&folder));
    pump();
    assert_eq!(
        pane.cursor_id(),
        Some(MessageId::new(1)),
        "the autoselect opens the folder on its first row"
    );

    // A search over the same account. The same length, so it is the same
    // shape of announcement as a reorder — and message 1 is a hit, so a pin
    // that went by identity alone has something to find.
    let hits = [4, 5, 1, 6, 2, 3];
    pane.model().set_source(Rc::new(Answer(hits.to_vec())));
    pane.model().deliver(0, rows(&hits));
    pump();

    assert_eq!(
        pane.cursor_id(),
        Some(MessageId::new(4)),
        "the results opened on the message the folder had been showing \
         instead of on the best match: the cursor was carried across a \
         change of what the list is answering"
    );

    window.close();
}
