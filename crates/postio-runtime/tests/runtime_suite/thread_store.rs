//! The threaded window, through the store the frontend actually holds
//! (ADR 0015, #307).
//!
//! `postio-storage`'s own tests cover the query. This covers the layer above
//! it: that a folder scope answers conversations, that a query view refuses
//! to, and that the seek marks the message window uses do not get confused by
//! a second window over the same folder.

use postio_model::mailbox::MailboxRole;
use postio_model::{AccountId, MailboxId};
use postio_runtime::store::{ListScope, LocalStore, MailStore, PageRequest};
use postio_storage::seed::{seed_large, thread_seeded_messages};
use postio_storage::test_support;

async fn store(
    messages: usize,
    per_thread: usize,
) -> (LocalStore, AccountId, MailboxId, test_support::TempStore) {
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, messages).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, per_thread).await;
    let store = LocalStore::new(&database);
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
    let store = LocalStore::new(&database);

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
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 600).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4).await;
    let store = LocalStore::new(&database);

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
async fn paging_a_folder_opens_one_connection_rather_than_one_per_page() {
    // #1602: the engine keeps no pool, so a read that opens its own
    // connection pays a fresh pager, an empty page cache and the pragmas,
    // and a list page opened two. Counted, not timed, like the folder count
    // above: five pages of one folder should not cost five caches.
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 600).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4).await;
    let store = LocalStore::new(&database);

    let first = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("the first page");
    assert!(first.total > 0, "the fixture has to have rows");

    let before = test_support::counting::checkouts();
    for page in 1..6u32 {
        store
            .thread_page(request(ListScope::Mailbox(inbox), page * 50, 50))
            .await
            .expect("a later page");
    }
    let opened = test_support::counting::checkouts() - before;
    assert_eq!(
        opened, 0,
        "five pages of one folder opened {opened} more connections, each a \
         fresh page cache over the file; the first page's connection should \
         have served them all"
    );
}

#[tokio::test]
async fn the_rows_for_a_changed_message_are_the_page_row_they_replace() {
    // #1607: a mark-read re-read the whole page to learn one conversation's
    // new state. `rows_in` answers the same ids with the same row shape the
    // page uses, so the list can patch the row in place.
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 600).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4).await;
    let store = LocalStore::new(&database);

    let page = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("the first page");
    let shown = page.rows.first().expect("a row").clone();

    // The representative goes unread.
    {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::MessageRepository::new(&connection)
            .set_flags(
                shown.representative.id,
                &postio_model::FlagSet::from_iter(std::iter::empty::<postio_model::Flag>()),
                postio_storage::repository::FlagSource::Local,
            )
            .await
            .expect("unread");
    }

    let rows = store
        .rows_in(ListScope::Mailbox(inbox), vec![shown.representative.id])
        .await
        .expect("the rows for one id");
    let postio_runtime::store::ListRows::Threads(rows) = rows else {
        panic!("a folder lists conversations, so its rows are conversations");
    };
    assert_eq!(rows.len(), 1, "one id, one conversation: {rows:?}");
    let fresh = &rows[0];
    assert_eq!(fresh.id, shown.id);
    assert_eq!(fresh.representative.id, shown.representative.id);
    assert_eq!(fresh.message_count, shown.message_count);
    assert_eq!(fresh.participants, shown.participants);
    assert_eq!(
        fresh.unread_count,
        shown.unread_count + u32::from(shown.representative.seen),
        "the row carries the flag that moved"
    );
}

#[tokio::test]
async fn a_folder_told_about_an_archive_is_not_counted_again() {
    // #1607: an archive moved the folder's `total_count`, which is the
    // count's witness, so the next page paid the whole conversation count
    // again -- 786 ms on a real 60k folder, in front of the first row. Told
    // which messages left, the store adjusts the count it holds by the
    // conversations that actually left and keeps serving it.
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 300).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let elsewhere = report
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.id != inbox)
        .expect("another folder")
        .id;
    // Conversations of one, so archiving any row's message removes the row.
    thread_seeded_messages(&database, report.account.id, 1).await;
    let store = LocalStore::new(&database);
    let first = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("a page");
    let alone = first
        .rows
        .iter()
        .find(|row| row.message_count == 1)
        .expect("a conversation of one on the first page")
        .representative
        .id;
    {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::MessageRepository::new(&connection)
            .move_to(&[alone], elsewhere)
            .await
            .expect("archived");
    }
    store.note_removed(inbox, vec![alone]);

    let before = postio_runtime::store::folders_counted();
    let after = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("a page after the archive");
    assert_eq!(
        postio_runtime::store::folders_counted() - before,
        0,
        "the folder was told what left and counted itself again anyway"
    );
    assert_eq!(
        after.total,
        first.total - 1,
        "the archived row is still counted: {} then {}",
        first.total,
        after.total
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
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 300).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4).await;
    let store = LocalStore::new(&database);

    let first = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("a page");

    // New mail, through the repository the sync uses, so the triggers run.
    {
        let connection = database.connect().await.expect("a connection");
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
            .await
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

#[tokio::test]
async fn a_message_list_s_rows_cost_the_same_however_many() {
    // #1613: each message row read one `threads.get` for its conversation's
    // size -- fifty-one statements for a page of fifty, in search results
    // and every query view. The size is a column of the row's own query.
    use postio_storage::test_support::counting::counted_async;
    let (store, account, _inbox, database) = store(200, 4).await;
    let ids: Vec<postio_model::MessageId> = {
        let connection = database.connect().await.expect("a connection");
        postio_storage::sql::execute(
            &connection,
            "UPDATE messages SET flagged = 1 WHERE account_id = ?1",
            [account.get()],
        )
        .await
        .expect("flag them");
        postio_storage::sql::all(
            &connection,
            "SELECT id FROM messages WHERE thread_id IS NOT NULL ORDER BY id LIMIT 50",
            (),
            |row| {
                Ok(postio_model::MessageId::new(
                    postio_storage::sql::RowExt::col(row, 0)?,
                ))
            },
        )
        .await
        .expect("ids")
    };
    let scope = ListScope::Flagged(account);
    let one = counted_async(|| async {
        store
            .rows_in(scope, ids[..1].to_vec())
            .await
            .expect("one row");
    })
    .await;
    let fifty = counted_async(|| async {
        let rows = store.rows_in(scope, ids.clone()).await.expect("fifty rows");
        let postio_runtime::store::ListRows::Messages(rows) = rows else {
            panic!("Flagged lists messages");
        };
        assert!(
            rows.iter().all(|row| row.thread_count >= 1),
            "every row says how big its conversation is"
        );
        assert!(
            rows.iter().any(|row| row.thread_count > 1),
            "the seeded conversations hold more than one message"
        );
    })
    .await;
    assert_eq!(
        one.statements, fifty.statements,
        "the rows of fifty messages took {} statements where one took {}",
        fifty.statements, one.statements
    );
}

#[tokio::test]
async fn the_unified_list_is_counted_once_while_nothing_moves() {
    // #1610: the unified view counted every conversation of every account on
    // every page -- the folder count's correlated shape, over all of them --
    // where a folder is counted once and kept while its witness holds.
    let (store, account, inbox, database) = store(300, 3).await;
    let before = postio_runtime::store::unified_counted();
    let mut total = 0;
    for page in 0..3 {
        total = store
            .thread_page(request(ListScope::Unified, page * 20, 20))
            .await
            .expect("a unified page")
            .total;
    }
    assert_eq!(
        postio_runtime::store::unified_counted() - before,
        1,
        "three pages of an unchanged unified list counted it more than once"
    );

    // And mail arriving is counted, not served from what was true before.
    {
        let connection = database.connect().await.expect("a connection");
        let mut message = postio_model::Message::new(account, inbox, chrono::Utc::now());
        message.subject = Some("a new conversation".to_owned());
        let id = postio_storage::repository::MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        // In a conversation of its own, as threading files every delivery.
        let mut thread = postio_model::Thread::new(account);
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        threads.create(&mut thread).await.expect("a thread");
        threads.add_message(thread.id, id).await.expect("joined");
    }
    let after = store
        .thread_page(request(ListScope::Unified, 0, 20))
        .await
        .expect("a unified page")
        .total;
    assert_eq!(after, total + 1, "a new conversation went uncounted");
}

#[tokio::test]
async fn a_jump_to_the_bottom_of_a_folder_seeks_rather_than_skips() {
    // #1610: a scrollbar drag into a large folder read its page with an
    // OFFSET over the window's correlated predicate, linear in the depth.
    let (store, account, inbox, database) = store(3_000, 3).await;
    let total = store
        .thread_count(ListScope::Mailbox(inbox))
        .await
        .expect("a count");
    assert!(total > 400, "a folder long enough to jump in: {total}");
    let offset = total - 20;
    let page = store
        .thread_page(request(ListScope::Mailbox(inbox), offset, 50))
        .await
        .expect("the bottom page");
    assert!(
        postio_runtime::store::last_thread_skip() <= 50,
        "the jump skipped {} rows after its seek",
        postio_runtime::store::last_thread_skip()
    );

    // And it is the page OFFSET would have read.
    let connection = database.connect().await.expect("a connection");
    let mut query = postio_storage::repository::ThreadListQuery::in_mailbox(account, inbox);
    query.limit = 50;
    let expected = postio_storage::repository::ThreadRepository::new(&connection)
        .page_at(&query, offset)
        .await
        .expect("by offset");
    assert_eq!(
        page.rows
            .iter()
            .map(|row| row.representative.id)
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|row| row.latest.as_ref().expect("a representative").id)
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn a_query_view_is_counted_from_the_folders_cached_counts() {
    // #1614: the Flagged and Snoozed pages ran a count(*) over messages on
    // every page read -- `flagged` is in no index, so the
    // Flagged count read the table row of every message in the account. The
    // sidebar already sums the folders' cached columns for the same numbers.
    // A sentinel in the column is what proves the page read it rather than
    // counting the messages again.
    let (store, account, _inbox, database) = store(200, 4).await;
    {
        let connection = database.connect().await.expect("a connection");
        postio_storage::sql::execute(
            &connection,
            "UPDATE mailboxes SET flagged_count = 0, snoozed_count = 0 WHERE account_id = ?1",
            [account.get()],
        )
        .await
        .expect("clear");
        postio_storage::sql::execute(
            &connection,
            "UPDATE mailboxes SET flagged_count = 777, snoozed_count = 4321
              WHERE id = (SELECT min(id) FROM mailboxes WHERE account_id = ?1)",
            [account.get()],
        )
        .await
        .expect("a sentinel");
    }
    for (scope, expected) in [
        (ListScope::Flagged(account), 777),
        (ListScope::Snoozed(account), 4321),
    ] {
        let total = match store
            .list_page(request(scope, 0, 10))
            .await
            .expect("a page")
        {
            postio_runtime::store::ListPage::Messages(page) => page.total,
            postio_runtime::store::ListPage::Threads(page) => page.total,
        };
        assert_eq!(total, expected, "{scope:?} was counted rather than read");
    }
}

#[tokio::test]
async fn archiving_from_an_inbox_takes_the_row_out_of_unified_and_its_count() {
    // #1692: Unified is the inboxes. An archive moves a message between two
    // folders and leaves every folder's total summed together where it was,
    // so a count held against that sum would still include the row -- and a
    // list told there are more rows than its pages hold draws placeholders
    // that never resolve.
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 300).await;
    let archive = report.mailbox(MailboxRole::Archive).expect("an archive").id;
    thread_seeded_messages(&database, report.account.id, 1).await;
    let store = LocalStore::new(&database);

    let first = store
        .thread_page(request(ListScope::Unified, 0, 20))
        .await
        .expect("a unified page");
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    assert_eq!(
        first.total,
        store
            .thread_count(ListScope::Mailbox(inbox))
            .await
            .expect("the inbox's count"),
        "one account: Unified is its inbox, row for row"
    );
    let top = first.rows[0].representative.id;

    {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::MessageRepository::new(&connection)
            .move_to(&[top], archive)
            .await
            .expect("archive the top row");
    }
    let after = store
        .thread_page(request(ListScope::Unified, 0, 20))
        .await
        .expect("a unified page");
    assert_eq!(
        after.total,
        first.total - 1,
        "the archived row is still counted"
    );
    assert!(
        after.rows.iter().all(|row| row.representative.id != top),
        "the archived row is still listed"
    );
}
