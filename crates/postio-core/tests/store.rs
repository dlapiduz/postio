#![cfg(feature = "runtime")]
//! Skipped without the `runtime` feature: the SQLite store is the half of
//! `postio-core` that owns a database, and it is off by default so that
//! `postio-gtk` never has `rusqlite` in its dependency graph.

//! Reading the local store through the runtime's own boundary.
//!
//! Everything here goes through [`Store`], which is what a frontend can reach:
//! no `rusqlite` types cross it, and no call blocks the thread that made it.
//! Nothing here touches the network.

use postio_core::store::{ListScope, MailStore, PageRequest, SqliteStore};
use postio_model::MailboxRole;
use postio_storage::seed::seed_small;
use postio_storage::test_support;

/// The seeded database, and a store over it.
fn seeded() -> (postio_storage::Database, postio_storage::seed::SeedReport) {
    let database = test_support::memory();
    let report = seed_small(&database, 7);
    (database, report)
}

#[tokio::test]
async fn a_page_carries_the_count_it_was_read_against() {
    let (database, report) = seeded();
    let store = SqliteStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let page = store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("the inbox reads");

    assert!(page.total > 0, "the seed put mail in the inbox");
    assert_eq!(
        page.rows.len() as u32,
        page.total.min(50),
        "a page holds what the count says is there, up to the limit"
    );
    assert_eq!(
        page.total,
        store
            .message_count(ListScope::Mailbox(inbox))
            .await
            .expect("the count reads"),
        "the count that came with the page is the count on its own"
    );
}

#[tokio::test]
async fn rows_come_newest_first_and_paging_walks_them_without_repeating() {
    let (database, report) = seeded();
    let store = SqliteStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let page = |offset, limit| {
        let store = store.clone();
        async move {
            store
                .message_page(PageRequest {
                    scope: ListScope::Mailbox(inbox),
                    offset,
                    limit,
                })
                .await
                .expect("the inbox reads")
        }
    };

    let whole = page(0, 100).await;
    assert!(
        whole
            .rows
            .windows(2)
            .all(|pair| pair[0].received_at >= pair[1].received_at),
        "the list is reverse-chronological"
    );

    let first = page(0, 3).await;
    let second = page(3, 3).await;
    assert_eq!(first.rows.len(), 3);
    assert_eq!(
        first.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        whole.rows[..3].iter().map(|row| row.id).collect::<Vec<_>>(),
    );
    assert_eq!(
        second.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        whole.rows[3..6]
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        "the second page resumes where the first stopped"
    );
}

#[tokio::test]
async fn every_row_knows_how_long_its_thread_is() {
    // The badge on the canvas' row is a count of the thread, and a source
    // that left it at one would silently remove the badge from every row.
    let (database, report) = seeded();
    let store = SqliteStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let page = store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: 100,
        })
        .await
        .expect("the inbox reads");

    assert!(
        page.rows.iter().all(|row| row.thread_count >= 1),
        "a message is at least its own thread"
    );
    assert!(
        page.rows.iter().any(|row| row.thread.is_some()),
        "the seed threaded something, or this proves nothing"
    );
}

#[tokio::test]
async fn a_page_past_the_end_is_empty_rather_than_an_error() {
    let (database, report) = seeded();
    let store = SqliteStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let page = store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 100_000,
            limit: 50,
        })
        .await
        .expect("reading past the end is a short answer, not a failure");

    assert!(page.rows.is_empty());
    assert!(page.total > 0, "and the count is still the real one");
}

#[tokio::test]
async fn the_account_reads_its_folders_with_their_counts() {
    let (database, report) = seeded();
    let store = SqliteStore::new(&database);

    let mailboxes = store
        .mailboxes(report.account.id)
        .await
        .expect("the folders read");

    assert_eq!(mailboxes.len(), report.mailboxes.len());
    let inbox = mailboxes
        .iter()
        .find(|mailbox| mailbox.role == MailboxRole::Inbox)
        .expect("an inbox");
    assert!(
        inbox.counts.total > 0,
        "the sidebar's count came back with the folder"
    );
}

#[tokio::test]
async fn several_reads_at_once_do_not_wedge_a_single_threaded_runtime() {
    // `#[tokio::test]` runs on a current-thread runtime: exactly one worker.
    // A read that ran *on* that worker rather than on a blocking thread would
    // hold it for the length of the query, and this is the shape that shows
    // it — several in flight, all expected back.
    let (database, report) = seeded();
    let store = SqliteStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let page = |offset| {
        let store = store.clone();
        async move {
            store
                .message_page(PageRequest {
                    scope: ListScope::Mailbox(inbox),
                    offset,
                    limit: 5,
                })
                .await
        }
    };

    let (first, second, third, fourth) = tokio::join!(page(0), page(5), page(10), page(15));
    for answer in [first, second, third, fourth] {
        answer.expect("every read comes back");
    }
}
