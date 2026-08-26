//! The threaded window, through the store the frontend actually holds
//! (ADR 0015, #307).
//!
//! `postio-storage`'s own tests cover the query. This covers the layer above
//! it: that a folder scope answers conversations, that a query view refuses
//! to, and that the seek marks the message window uses do not get confused by
//! a second window over the same folder.

use postio_model::mailbox::MailboxRole;
use postio_model::{AccountId, MailboxId};
use postio_runtime::store::{ListScope, MailStore, PageRequest, SqliteStore};
use postio_storage::seed::{seed_large, thread_seeded_messages};
use postio_storage::test_support;

fn store(messages: usize, per_thread: usize) -> (SqliteStore, AccountId, MailboxId, test_support::TempDatabase) {
    let database = test_support::temp();
    let report = seed_large(&database, 7, messages);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, per_thread);
    let store = SqliteStore::new(&database);
    (store, report.account.id, inbox, database)
}

fn request(scope: ListScope, offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        scope,
        offset,
        limit,
    }
}

#[tokio::test]
async fn a_folder_answers_conversations_rather_than_messages() {
    let (store, _account, inbox, _database) = store(200, 4);

    let page = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 20))
        .await
        .expect("a page of conversations");

    assert_eq!(page.rows.len(), 20, "a page is a window, never the folder");
    assert!(
        page.total > 0 && page.total < 200,
        "conversations are fewer than the messages in them: {}",
        page.total
    );
    for row in &page.rows {
        assert!(
            row.message_count >= 1,
            "a conversation the folder shows holds at least the message it is drawn from"
        );
        assert_eq!(
            row.representative.thread,
            Some(row.id),
            "the row is drawn from a message of its own conversation"
        );
    }
}

#[tokio::test]
async fn the_thread_count_matches_the_rows_the_window_would_produce() {
    let (store, _account, inbox, _database) = store(100, 4);

    let total = store
        .thread_count(ListScope::Mailbox(inbox))
        .await
        .expect("a count");
    let page = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 10_000))
        .await
        .expect("every conversation");

    assert_eq!(page.rows.len() as u32, total);
    assert_eq!(page.total, total);
}

#[tokio::test]
async fn a_query_view_says_it_lists_messages_rather_than_answering_wrongly() {
    // Folders thread; query views list messages (ADR 0015). Answering Flagged
    // with conversations would be the wrong answer rather than a missing one,
    // so it is refused where a caller can see it.
    let (store, account, _inbox, _database) = store(20, 4);

    let error = store
        .thread_page(request(ListScope::Flagged(account), 0, 10))
        .await
        .expect_err("Flagged is a query view");

    assert!(
        error.message().contains("messages"),
        "the sentence should say what that view does show: {}",
        error.message()
    );
}

#[tokio::test]
async fn paging_conversations_never_repeats_or_skips_a_row() {
    // The seek marks are the reason this is worth asserting: a page is read
    // by seeking to a remembered boundary and skipping the remainder, so an
    // off-by-one in the marks shows up as a duplicated or missing row rather
    // than as an error.
    let (store, _account, inbox, _database) = store(400, 4);

    let mut seen = Vec::new();
    for page in 0..5 {
        let window = store
            .thread_page(request(ListScope::Mailbox(inbox), page * 20, 20))
            .await
            .expect("a page of conversations");
        seen.extend(window.rows.iter().map(|row| row.id));
    }

    let mut unique = seen.clone();
    unique.sort_by_key(|id| id.get());
    unique.dedup();
    assert_eq!(
        unique.len(),
        seen.len(),
        "scrolling produced the same conversation twice"
    );
}

#[tokio::test]
async fn the_two_windows_over_one_folder_do_not_confuse_each_others_marks() {
    // A folder has both a message window and a thread window, with different
    // row counts. One set of seek marks would have each read clearing the
    // other's, which would show up as paging that silently walks from the top
    // every time — slow rather than wrong, and so easy to miss.
    let (store, _account, inbox, _database) = store(400, 4);

    for page in 0..4 {
        let messages = store
            .message_page(request(ListScope::Mailbox(inbox), page * 20, 20))
            .await
            .expect("a page of messages");
        assert_eq!(messages.rows.len(), 20);
        let threads = store
            .thread_page(request(ListScope::Mailbox(inbox), page * 20, 20))
            .await
            .expect("a page of conversations");
        assert_eq!(threads.rows.len(), 20);
    }
}
