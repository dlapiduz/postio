//! A search result set in the message list, without a database or a display.
//!
//! `postio-5w1.1`: every search surface postio-gtk exposes was wired in
//! `postio-1ag` except the results themselves, because there was no seam for
//! them. Searching showed a hit count and a preview of the best match while
//! the list underneath went on showing the folder, and there was no way to
//! walk what had been found.
//!
//! The seam is a second kind of read on the one `Feed`: a page of an
//! explicit list of ids, which is what a result set actually is — ranked,
//! possibly spanning folders, with no offset into anything. A separate
//! source object was the other option and would have duplicated the list
//! handle, the generation counter and the error handlers, all of which a
//! result set needs exactly as much as a mailbox does.
//!
//! One test function, for the reason `feed.rs` gives: these futures are
//! awaited on the thread-default main context, and two test threads driving
//! one context is a thing glib refuses loudly and only sometimes.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::glib;
use gtk::prelude::*;
use postio_core::Event;
use postio_gtk::feed::{
    Feed, FeedScope, MessageSource, Page, PageFuture, PageRequest, ResultSource, RowsFuture,
};
use postio_gtk::list::{MessageList, MessageRow, PAGE_SIZE, Row};
use postio_model::ids::{MailboxId, MessageId};

const INBOX: i64 = 1;

/// How many hits the search found.
///
/// Several pages of them, because the whole point of the seam is that a
/// result set is windowed like a mailbox and a fixture fitting in one page
/// would prove nothing. Not a multiple of `PAGE_SIZE`, so the last page is
/// short and the clamp that stops it asking for ids nobody matched is
/// actually exercised.
const HITS: usize = 237;

/// Answers both kinds of read, and remembers every question.
#[derive(Default)]
struct Fake {
    mailbox_asked: RefCell<Vec<PageRequest>>,
    /// Each entry is one batch of ids the result source was asked for.
    hits_asked: RefCell<Vec<Vec<MessageId>>>,
    total: RefCell<u32>,
}

/// The folder a request names. These tests open folders, not smart folders —
/// `gtk_flagged.rs` is where the query scope is exercised.
fn scope_mailbox(request: &PageRequest) -> MailboxId {
    request
        .scope
        .mailbox()
        .expect("these tests open folders, not queries")
}

impl Fake {
    fn holding(total: u32) -> Rc<Self> {
        Rc::new(Fake {
            total: RefCell::new(total),
            ..Fake::default()
        })
    }

    fn drain_mailbox(&self) -> Vec<PageRequest> {
        self.mailbox_asked.borrow_mut().drain(..).collect()
    }

    fn drain_hits(&self) -> Vec<Vec<MessageId>> {
        self.hits_asked.borrow_mut().drain(..).collect()
    }
}

impl MessageSource for Fake {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        self.mailbox_asked.borrow_mut().push(request);
        let total = *self.total.borrow();
        let mailbox = scope_mailbox(&request);
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| folder_row(mailbox, position))
                .collect();
            Ok(Page { total, rows })
        })
    }
}

impl ResultSource for Fake {
    fn rows(&self, ids: Vec<MessageId>) -> RowsFuture {
        self.hits_asked.borrow_mut().push(ids.clone());
        Box::pin(async move { Ok(ids.into_iter().map(hit_row).collect()) })
    }
}

/// A row from the folder. Ids are far from the hits' so a subject is not the
/// only thing telling them apart.
fn folder_row(mailbox: MailboxId, position: u32) -> Row {
    row(
        MessageId::new(900_000 + position as i64),
        format!("mailbox {} message {position}", mailbox.get()),
    )
}

/// A row for a hit, keyed by the id the result source was asked for, so the
/// test can assert the list is showing *these* messages in *this* order.
fn hit_row(id: MessageId) -> Row {
    let subject = format!("hit {}", id.get());
    row(id, subject)
}

fn row(id: MessageId, subject: String) -> Row {
    Row {
        id,
        thread: None,
        from: None,
        subject: Some(subject),
        preview: None,
        received_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        seen: false,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 1,
    }
}

/// The hits, most relevant first. Ids descend so that "in rank order" and
/// "in id order" are different claims.
fn hits(count: usize) -> Vec<MessageId> {
    (0..count)
        .map(|rank| MessageId::new(1_000 + (count - rank) as i64))
        .collect()
}

fn results(messages: Vec<MessageId>) -> Event {
    Event::SearchResults {
        query: "from:ada invoice".to_string(),
        messages,
        took: std::time::Duration::from_millis(3),
    }
}

/// Let every future that is ready to finish, finish.
fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

fn loaded(list: &MessageList, position: u32) -> Option<Row> {
    list.item(position)
        .and_downcast::<MessageRow>()
        .and_then(|item| item.row())
}

fn subject_at(list: &MessageList, position: u32) -> Option<String> {
    loaded(list, position).and_then(|row| row.subject)
}

#[test]
fn search_hits_reach_the_message_list() {
    let found = hits(HITS);

    // ── a feed with no result source goes on showing the folder ─────────────
    // Not every window has a search wired to it, and one that does not should
    // ignore the event rather than empty its list.
    {
        let source = Fake::holding(4_000);
        let list = MessageList::new();
        let feed = Feed::new(&list, source.clone());
        feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
        settle();
        source.drain_mailbox();

        feed.apply(&results(found.clone()));
        settle();

        assert!(!feed.showing_results());
        assert_eq!(list.n_items(), 4_000, "the folder is still what is showing");
        assert!(source.drain_hits().is_empty());
    }

    let source = Fake::holding(4_000);
    let list = MessageList::new();
    let feed = Feed::new(&list, source.clone());
    feed.set_result_source(source.clone());

    feed.open(FeedScope::Mailbox(MailboxId::new(INBOX)));
    settle();
    assert_eq!(list.n_items(), 4_000);
    source.drain_mailbox();

    // ── the hits become the list ────────────────────────────────────────────
    feed.apply(&results(found.clone()));

    assert!(feed.showing_results());
    assert_eq!(
        feed.mailbox(),
        Some(MailboxId::new(INBOX)),
        "the mailbox is remembered, not left -- Esc goes back to it"
    );
    // The count is known the moment the ids arrive: they *are* the answer, so
    // unlike a mailbox there is nothing to wait for before saying how many.
    assert_eq!(list.n_items(), HITS as u32);

    settle();
    assert_eq!(
        subject_at(&list, 0),
        Some(format!("hit {}", found[0].get())),
        "the first row is the best match"
    );
    assert_eq!(
        subject_at(&list, 1),
        Some(format!("hit {}", found[1].get())),
        "and the second is the second best, in rank order"
    );

    // ── and only a page of them is read ─────────────────────────────────────
    let asked = source.drain_hits();
    assert_eq!(asked.len(), 1, "showing results read more than once");
    assert_eq!(
        asked[0],
        found[..PAGE_SIZE as usize].to_vec(),
        "the first page of hits, in rank order, and nothing else"
    );
    assert!(
        source.drain_mailbox().is_empty(),
        "a search read the folder as well as the index"
    );

    // Two hundred and fifty hits, one page of rows.
    assert!(
        list.resident_rows() <= PAGE_SIZE as usize * 3,
        "{} rows resident over a result set",
        list.resident_rows()
    );

    // ── walking past the first page reads the next one, and only it ─────────
    let deep = HITS as u32 - 1;
    let _ = list.item(deep);
    settle();

    // The page holding it, and the one before -- `MessageList::row_at` asks
    // for the neighbours too, so scrolling at speed does not stall on a page
    // boundary. There is no page after this one to ask for.
    let asked = source.drain_hits();
    let page = deep / PAGE_SIZE;
    let start = (page * PAGE_SIZE) as usize;
    assert_eq!(
        asked,
        vec![
            found[start..].to_vec(),
            found[start - PAGE_SIZE as usize..start].to_vec(),
        ],
        "reading one position read the wrong pages"
    );
    assert_eq!(
        asked[0].len(),
        HITS % PAGE_SIZE as usize,
        "the last page is short, and asked for ids nobody matched"
    );
    assert_eq!(
        subject_at(&list, deep),
        Some(format!("hit {}", found[deep as usize].get())),
        "the cursor can reach the last hit"
    );

    // ── mail arriving in the folder does not push the results down ──────────
    // `NewMail` means the mailbox got longer. The result set did not, and a
    // list that inserted at the top of it would put a message that does not
    // match the query above one that does.
    let before = list.n_items();
    feed.apply(&Event::NewMail {
        mailbox: MailboxId::new(INBOX),
        messages: vec![MessageId::new(999_001), MessageId::new(999_002)],
    });
    settle();
    assert_eq!(list.n_items(), before, "new mail grew the result set");
    assert_eq!(
        subject_at(&list, 0),
        Some(format!("hit {}", found[0].get())),
        "new mail displaced the best match"
    );

    // ── a flag change on a hit still repaints it ────────────────────────────
    // The rows are still the same rows in the same order, so this is the one
    // mailbox-shaped event a result set does care about.
    source.drain_hits();
    feed.apply(&Event::MessagesChanged {
        messages: vec![found[0]],
    });
    settle();
    let asked = source.drain_hits();
    assert_eq!(
        asked.len(),
        1,
        "a flag change refetched the wrong number of pages"
    );
    assert_eq!(asked[0], found[..PAGE_SIZE as usize].to_vec());

    // ── Esc puts the folder back, without a flash of nothing ────────────────
    assert!(feed.close_results(), "there were results to close");
    assert!(!feed.showing_results());

    // Before anything is read back. The list already knows how long the
    // mailbox is -- it has not changed -- and zeroing it here would collapse
    // the scroller to nothing and make the offset the window restores
    // meaningless.
    assert_eq!(
        list.n_items(),
        4_000,
        "returning to the folder emptied it first"
    );

    settle();
    assert_eq!(
        subject_at(&list, 0),
        Some("mailbox 1 message 0".to_string()),
        "the folder's own mail is back"
    );
    assert!(
        !source.drain_mailbox().is_empty(),
        "returning to the folder never re-read it"
    );

    // Closing again is what a second Esc does, and it is not an error.
    assert!(!feed.close_results());
}
