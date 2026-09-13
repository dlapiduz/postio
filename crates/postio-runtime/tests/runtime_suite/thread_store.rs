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

async fn store(
    messages: usize,
    per_thread: usize,
) -> (SqliteStore, AccountId, MailboxId, test_support::TempStore) {
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, messages).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, per_thread).await;
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
    let (store, _account, inbox, _database) = store(200, 4).await;

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
            row.representative.thread, row.id,
            "the row is drawn from a message of its own conversation"
        );
    }
}

#[tokio::test]
async fn the_thread_count_matches_the_rows_the_window_would_produce() {
    let (store, _account, inbox, _database) = store(100, 4).await;

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
    let (store, account, _inbox, _database) = store(20, 4).await;

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
    let (store, _account, inbox, _database) = store(400, 4).await;

    let mut seen: Vec<postio_model::ids::MessageId> = Vec::new();
    for page in 0..5 {
        let window = store
            .thread_page(request(ListScope::Mailbox(inbox), page * 20, 20))
            .await
            .expect("a page of conversations");
        // By representative, because an unthreaded message is a row with no
        // thread id and two of them must still be two rows.
        seen.extend(window.rows.iter().map(|row| row.representative.id));
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
    let (store, _account, inbox, _database) = store(400, 4).await;

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

/// The unified scope is a list over every account, and the store has to page
/// it by position like any other.
///
/// The risk this covers is the offset bridge, not the grouping —
/// `postio-storage` owns that. `read_thread_page` remembers a cursor for a
/// row it has already handed out and then skips forward from it, and the
/// unified walk folds threads together as it goes, so an off-by-one in that
/// bridge shows up as a row served twice or a row never served at all.
/// Walking the whole list and insisting every row is distinct is what would
/// catch it.
#[tokio::test]
async fn the_unified_scope_pages_every_account_without_repeating_a_row() {
    let database = test_support::temp().await;
    postio_storage::seed::seed_small(&database, 3).await;
    postio_storage::seed::seed_extra_account(&database, "Second", "grace@example.org", 4).await;
    let store = SqliteStore::new(&database);

    let first = store
        .thread_page(request(ListScope::Unified, 0, 10))
        .await
        .expect("a unified page");
    let total = first.total;
    assert!(
        total > 10,
        "the fixture has to span more than one page or the skip is never \
         exercised at all; got {total}"
    );

    let mut seen: Vec<postio_model::MessageId> = Vec::new();
    let mut offset = 0;
    while offset < total {
        let page = store
            .thread_page(request(ListScope::Unified, offset, 10))
            .await
            .expect("a unified page");
        assert!(
            !page.rows.is_empty(),
            "row {offset} of {total} is inside the list and has to be servable"
        );
        assert_eq!(
            page.total, total,
            "the total cannot move while nothing is being written"
        );
        offset += page.rows.len() as u32;
        seen.extend(page.rows.iter().map(|row| row.representative.id));
    }

    assert_eq!(
        seen.len(),
        total as usize,
        "the walk serves exactly the number of rows it promised"
    );
    let mut distinct = seen.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        seen.len(),
        "no conversation is served twice: absorption means a group is not a \
         fixed number of threads, so the skip has to count rows"
    );
}

#[tokio::test]
async fn paging_a_folder_counts_it_once_rather_than_once_per_page() {
    // #1534. `read_thread_page` recomputes `count_of` on every page, and for
    // a threaded folder that is not a column lookup — it is a correlated
    // subquery, one indexed probe per message. Measured on a real account:
    // 786 ms for a 60,907-message Archive, against a 100 ms interaction
    // budget. Paid again for every page the list asks for, which is what made
    // the folder impossible to open.
    //
    // `Marks`' own doc comment states the assumption this breaks: "The total
    // is checked on every read — it is a column lookup now, not a count."
    // True of the message window, whose total is the trigger-maintained
    // column. Never true of this one.
    //
    // Counted, not timed: the count is a statement, and statements are what
    // `postio_storage`'s trace hook sees. Six pages of the same folder should
    // not cost six counts.
    let database = test_support::temp();
    let report = seed_large(&database, 7, 600);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4);
    let store = SqliteStore::new(&database);

    // One page, to establish what a page costs including its first count.
    let first = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("the first page");
    assert!(first.total > 0, "the fixture has to have rows");

    // Five more pages of the same folder, nothing changing underneath.
    let before = postio_runtime::store::folders_counted();
    for page in 1..6u32 {
        store
            .thread_page(request(ListScope::Mailbox(inbox), page * 50, 50))
            .await
            .expect("a later page");
    }
    let counted = postio_runtime::store::folders_counted() - before;

    // Each page legitimately issues its own read. What it must not issue is
    // its own count of the whole folder: the list has not changed, and the
    // answer is the one already in hand.
    //
    // Two statements per page is the honest budget — the page, and whatever
    // the scope resolution needs. Six counts on top of that is the defect.
    assert_eq!(
        counted, 0,
        "five pages of an unchanged folder counted it {counted} more times. A \
         folder is counted once, not once per page: on a real Archive that \
         count is 786 ms, so five pages spend four seconds recomputing a \
         number already in hand (#1534)."
    );
}

#[tokio::test]
async fn a_folder_that_gains_a_message_is_counted_again() {
    // The other half of caching the count: it has to stop being used the
    // moment it is wrong. A cached total that outlived its folder would make
    // the list claim a row count it cannot fill, which is a worse bug than
    // the one the cache fixes.
    //
    // The witness is `mailboxes.total_count`, maintained by the counting
    // triggers -- so this asserts the trigger and the cache agree, not just
    // that the cache has an invalidation path.
    let database = test_support::temp();
    let report = seed_large(&database, 7, 300);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4);
    let store = SqliteStore::new(&database);

    let first = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("a page");

    // New mail, through the repository the sync uses, so the triggers run.
    {
        let connection = database.connection().expect("a connection");
        let mut arrival = postio_model::Message::new(
            report.account.id,
            inbox,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        );
        arrival.subject = Some("A message that arrived after the count".to_owned());
        arrival.from = vec![postio_model::EmailAddress::new(
            Some("Lena"),
            "lena@example.com",
        )];
        postio_storage::repository::MessageRepository::new(&connection)
            .create(&mut arrival)
            .expect("the arrival");
    }

    let before = postio_runtime::store::folders_counted();
    let after = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("a page after the arrival");

    assert_eq!(
        postio_runtime::store::folders_counted() - before,
        1,
        "the folder changed and the count was served from the cache anyway"
    );
    assert_eq!(
        after.total,
        first.total + 1,
        "the new message did not reach the row count: {} then {}",
        first.total,
        after.total
    );
}
