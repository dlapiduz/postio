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
    let (store, _account, inbox, _database) = store(400, 4).await;

    let mut seen: Vec<postio_model::ids::MessageId> = Vec::new();
    for page in 0..5 {
        let window = store
            .thread_page(request(ListScope::Mailbox(inbox), page * 20, 20))
            .await
            .expect("a page of conversations");
        // By representative: an unthreaded message has no thread id, and two of them must still be
        // two rows.
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

/// Covers the offset bridge, not the grouping: an off-by-one in remembering a cursor and skipping
/// forward shows as a row served twice or never.
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
    if postio_test_support::alone!("paging_a_folder_counts_it_once_rather_than_once_per_page") {
        return;
    }
    // #1534: `count_of` was recomputed on every page and is a correlated subquery (786 ms on a
    // 60,907-message Archive).
    // Counted, not timed: six pages of one folder should not cost six counts.
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

    let before = postio_runtime::store::folders_counted();
    for page in 1..6u32 {
        store
            .thread_page(request(ListScope::Mailbox(inbox), page * 50, 50))
            .await
            .expect("a later page");
    }
    let counted = postio_runtime::store::folders_counted() - before;

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
    if postio_test_support::alone!("paging_a_folder_opens_one_connection_rather_than_one_per_page")
    {
        return;
    }
    // #1602: a read opening its own connection pays a fresh pager, page cache and pragmas. Counted,
    // not timed.
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
    if postio_test_support::alone!("a_folder_told_about_an_archive_is_not_counted_again") {
        return;
    }
    // #1607: an archive moved the folder's `total_count`, the count's witness, so the next page
    // paid the whole count again; told which messages left, the store adjusts instead.
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 300).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let elsewhere = report
        .mailboxes
        .iter()
        .find(|mailbox| mailbox.id != inbox)
        .expect("another folder")
        .id;
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
    if postio_test_support::alone!("a_folder_that_gains_a_message_is_counted_again") {
        return;
    }
    // A cached total must stop being used the moment it is wrong; the witness is the
    // trigger-maintained `total_count`, so this asserts the trigger and the cache agree.
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, 300).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    thread_seeded_messages(&database, report.account.id, 4).await;
    let store = LocalStore::new(&database);

    let first = store
        .thread_page(request(ListScope::Mailbox(inbox), 0, 50))
        .await
        .expect("a page");

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
    // #1613: each message row read one `threads.get` for its conversation size; the size is now a
    // column of the row's own query.
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
    if postio_test_support::alone!("the_unified_list_is_counted_once_while_nothing_moves") {
        return;
    }
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

    {
        let connection = database.connect().await.expect("a connection");
        let mut message = postio_model::Message::new(account, inbox, chrono::Utc::now());
        message.subject = Some("a new conversation".to_owned());
        let id = postio_storage::repository::MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
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
    if postio_test_support::alone!("a_jump_to_the_bottom_of_a_folder_seeks_rather_than_skips") {
        return;
    }
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
    // #1614: the Flagged and Snoozed pages ran `count(*)` per page; a sentinel in the cached column
    // proves the page read it instead.
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
    // #1692: an archive moves a message between folders and leaves their summed totals unchanged,
    // so a count held against that sum would still include the row.
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

const FOCUS: ListScope = ListScope::Focus(postio_model::FocusScope::Inbox);

#[tokio::test]
async fn focus_s_inbox_is_the_unified_inbox_for_now() {
    let (store, _account, _inbox, _database) = store(200, 4).await;
    let total = store.list_count(ListScope::Unified).await.expect("a count");
    assert!(
        total > 4,
        "the inbox has to span pages; it holds {total} rows"
    );
    assert_eq!(store.list_count(FOCUS).await.expect("a count"), total);
    for offset in (0..total).step_by(4) {
        let focus = store
            .thread_page(request(FOCUS, offset, 4))
            .await
            .expect("a Focus page");
        let unified = store
            .thread_page(request(ListScope::Unified, offset, 4))
            .await
            .expect("a unified page");
        assert!(
            !focus.rows.is_empty(),
            "row {offset} of {total} is servable"
        );
        assert_eq!(focus, unified, "the page at {offset}");
    }
}

#[tokio::test]
async fn focus_s_inbox_pages_every_account_without_repeating_a_row() {
    let database = test_support::temp().await;
    postio_storage::seed::seed_small(&database, 3).await;
    postio_storage::seed::seed_extra_account(&database, "Second", "grace@example.org", 4).await;
    let store = LocalStore::new(&database);

    let total = store
        .thread_page(request(FOCUS, 0, 10))
        .await
        .expect("a Focus page")
        .total;
    assert!(total > 10, "the fixture has to span pages; got {total}");
    let mut seen: Vec<postio_model::MessageId> = Vec::new();
    let mut offset = 0;
    while offset < total {
        let page = store
            .thread_page(request(FOCUS, offset, 10))
            .await
            .expect("a Focus page");
        assert!(!page.rows.is_empty(), "row {offset} of {total} is servable");
        assert_eq!(page.total, total, "nothing is being written");
        offset += page.rows.len() as u32;
        seen.extend(page.rows.iter().map(|row| row.representative.id));
    }
    let mut distinct = seen.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), seen.len(), "no row is served twice");
    assert_eq!(seen.len(), total as usize, "and the walk serves the total");
}

#[tokio::test]
async fn focus_s_inbox_is_counted_once_while_nothing_moves() {
    if postio_test_support::alone!("focus_s_inbox_is_counted_once_while_nothing_moves") {
        return;
    }
    let (store, account, inbox, database) = store(300, 3).await;
    let before = postio_runtime::store::focus_counted();
    let mut total = 0;
    for page in 0..3 {
        total = store
            .thread_page(request(FOCUS, page * 20, 20))
            .await
            .expect("a Focus page")
            .total;
    }
    assert_eq!(
        postio_runtime::store::focus_counted() - before,
        1,
        "three pages of an unchanged inbox counted it more than once"
    );

    {
        let connection = database.connect().await.expect("a connection");
        let mut message = postio_model::Message::new(account, inbox, chrono::Utc::now());
        message.subject = Some("a new conversation".to_owned());
        let id = postio_storage::repository::MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        let mut thread = postio_model::Thread::new(account);
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        threads.create(&mut thread).await.expect("a thread");
        threads.add_message(thread.id, id).await.expect("joined");
    }
    let after = store
        .thread_page(request(FOCUS, 0, 20))
        .await
        .expect("a Focus page");
    assert_eq!(after.total, total + 1, "a new conversation went uncounted");
    assert_eq!(
        after.rows[0].subject.as_deref(),
        Some("a new conversation"),
        "and it is the newest row"
    );
}

#[tokio::test]
async fn no_list_but_focus_s_carries_a_marker() {
    let (store, account, inbox, database) = store(200, 4).await;
    let top = store
        .thread_page(request(FOCUS, 0, 1))
        .await
        .expect("a Focus page")
        .rows
        .remove(0)
        .representative
        .id;
    {
        let connection = database.connect().await.expect("a connection");
        postio_storage::repository::MarkerRepository::new(&connection)
            .insert(&postio_storage::repository::Marker {
                message: top,
                kind: postio_model::listing::MarkerKind::Question,
                source: postio_storage::repository::MarkerSource::Detector,
                span: Some((0, 21)),
                excerpt: Some("Could you look at it?".to_owned()),
                starts_at: None,
                ends_at: None,
                due_at: None,
                invite: None,
                invite_state: None,
                answer: None,
                dismissed_at: None,
            })
            .await
            .expect("a marker");
    }
    let focus = store
        .thread_page(request(FOCUS, 0, 50))
        .await
        .expect("a Focus page");
    assert_eq!(
        focus.rows[0]
            .marker
            .as_ref()
            .and_then(|marker| marker.excerpt.as_deref()),
        Some("Could you look at it?"),
        "Focus's row draws its conversation's marker"
    );
    assert_eq!(
        focus.rows.iter().filter(|row| row.marker.is_some()).count(),
        1
    );
    for scope in [
        ListScope::Mailbox(inbox),
        ListScope::Account(account),
        ListScope::Unified,
    ] {
        let page = store
            .thread_page(request(scope, 0, 50))
            .await
            .expect("a page of conversations");
        assert!(!page.rows.is_empty(), "{scope:?}");
        assert!(
            page.rows.iter().all(|row| row.marker.is_none()),
            "{scope:?} drew a marker"
        );
    }
}

#[tokio::test]
async fn focus_s_inbox_folds_what_the_unified_inbox_folds() {
    let database = test_support::temp().await;
    postio_storage::seed::seed_small(&database, 3).await;
    postio_storage::seed::seed_extra_account(&database, "Second", "grace@example.org", 4).await;
    let store = LocalStore::new(&database);

    let total = store.list_count(FOCUS).await.expect("a count");
    assert_eq!(
        total,
        store.list_count(ListScope::Unified).await.expect("a count"),
        "Focus's inbox has the unified inbox's rows"
    );
    let mut folded = 0;
    let mut offset = 0;
    while offset < total {
        let focus = store
            .thread_page(request(FOCUS, offset, 10))
            .await
            .expect("a Focus page");
        let unified = store
            .thread_page(request(ListScope::Unified, offset, 10))
            .await
            .expect("a unified page");
        assert!(
            !focus.rows.is_empty(),
            "row {offset} of {total} is servable"
        );
        assert_eq!(focus, unified, "the page at {offset}");
        folded += focus
            .rows
            .iter()
            .filter(|row| !row.copies.is_empty())
            .count();
        offset += focus.rows.len() as u32;
    }
    assert!(
        folded > 0,
        "the fixture has to fold a conversation, or this proves nothing"
    );
}

#[tokio::test]
async fn holding_mail_moves_focus_s_total_marks_and_rows_together() {
    // A digest rule holding mail takes it out of Focus's inbox without any folder moving; the
    // count, seek marks and rows must still agree.
    let (store, _account, _inbox, database) = store(200, 4).await;
    let before = store
        .thread_page(request(FOCUS, 0, 20))
        .await
        .expect("a Focus page");
    let held = before.rows[0].id.expect("a conversation");
    {
        let connection = database.connect().await.expect("a connection");
        postio_storage::sql::execute(
            &connection,
            "INSERT INTO digest_holds (message_id, rule, held_at)
             SELECT id, 'Newsletters', 0 FROM messages WHERE thread_id = ?1",
            [held.get()],
        )
        .await
        .expect("held");
    }

    let after = store
        .thread_page(request(FOCUS, 0, 20))
        .await
        .expect("a Focus page");
    assert_eq!(
        after.total,
        before.total - 1,
        "the held conversation left the count"
    );
    let mut seen = Vec::new();
    let mut offset = 0;
    while offset < after.total {
        let page = store
            .thread_page(request(FOCUS, offset, 20))
            .await
            .expect("a Focus page");
        assert!(!page.rows.is_empty(), "row {offset} is servable");
        offset += page.rows.len() as u32;
        seen.extend(page.rows.into_iter().map(|row| row.id));
    }
    assert_eq!(seen.len() as u32, after.total, "the walk serves the total");
    assert!(
        !seen.contains(&Some(held)),
        "and never the held conversation"
    );
}
