use postio_model::MailboxRole;
use postio_runtime::store::{ListScope, LocalStore, MailStore, PageRequest};
use postio_storage::seed::seed_small;
use postio_storage::test_support;

async fn seeded() -> (postio_storage::Store, postio_storage::seed::SeedReport) {
    let database = test_support::memory().await;
    let report = seed_small(&database, 7).await;
    (database, report)
}

#[tokio::test]
async fn a_page_carries_the_count_it_was_read_against() {
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);
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
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let page = async |offset, limit| {
        store
            .message_page(PageRequest {
                scope: ListScope::Mailbox(inbox),
                offset,
                limit,
            })
            .await
            .expect("the inbox reads")
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
    // The badge is the thread's count; a source leaving it at one would silently remove it from
    // every row.
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);
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
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);
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
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);

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
    // A current-thread runtime has one worker: a read running on it would hold it for the whole
    // query.
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let page = async |offset| {
        store
            .message_page(PageRequest {
                scope: ListScope::Mailbox(inbox),
                offset,
                limit: 5,
            })
            .await
    };

    let (first, second, third, fourth) = tokio::join!(page(0), page(5), page(10), page(15));
    for answer in [first, second, third, fourth] {
        answer.expect("every read comes back");
    }
}

#[tokio::test]
async fn seeking_to_a_page_finds_the_same_rows_as_walking_to_it() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 3).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let walked = LocalStore::new(&database);
    let mut cold = Vec::new();
    for offset in (0..12).step_by(3) {
        cold.push(page_ids(&walked, inbox, offset, 3).await);
    }

    let sought = LocalStore::new(&database);
    let mut warm = Vec::new();
    for offset in (0..12).step_by(3) {
        warm.push(page_ids(&sought, inbox, offset, 3).await);
    }

    assert_eq!(cold, warm, "seeking and walking disagree about the rows");
    let flat: Vec<_> = warm.iter().flatten().collect();
    let unique: std::collections::BTreeSet<_> = flat.iter().collect();
    assert_eq!(flat.len(), unique.len(), "a page repeated a row: {warm:?}");
}

#[tokio::test]
async fn a_list_that_changed_length_throws_the_remembered_boundaries_away() {
    // A remembered boundary points at the wrong row when mail arrives or leaves; a changed count
    // drops the marks.
    // A same-length reorder is not caught, exactly as with plain `OFFSET`.
    let database = test_support::memory().await;
    let report = seed_small(&database, 5).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let store = LocalStore::new(&database);

    page_ids(&store, inbox, 0, 3).await;
    page_ids(&store, inbox, 3, 3).await;

    {
        let connection = database.connect().await.expect("a connection");
        connection
            .execute(
                "UPDATE messages SET deleted_locally = 1
                   WHERE id = (SELECT id FROM messages WHERE mailbox_id = ?1
                               ORDER BY received_at DESC, id DESC LIMIT 1)",
                [inbox.get()],
            )
            .await
            .expect("the fixture writes");
        postio_storage::repository::MailboxRepository::new(&connection)
            .recount(inbox)
            .await
            .expect("the count is kept up to date");
    }

    let after = page_ids(&store, inbox, 3, 3).await;
    let fresh = page_ids(&LocalStore::new(&database), inbox, 3, 3).await;
    assert_eq!(
        after, fresh,
        "a store holding stale boundaries disagreed with one reading cold"
    );
}

#[tokio::test]
async fn a_cached_count_of_zero_is_checked_rather_than_believed() {
    // The total is the list model's `n_items`, and a `GtkListView` over length zero asks for no
    // pages, so a count
    // wrong low draws an empty mailbox; the read must degrade to slow, not invisible.
    let database = test_support::memory().await;
    let report = seed_small(&database, 5).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    {
        let connection = database.connect().await.expect("a connection");
        // Behind the triggers' back: the drift being guarded against.
        connection
            .execute(
                "UPDATE mailboxes SET total_count = 0 WHERE id = ?1",
                [inbox.get()],
            )
            .await
            .expect("the fixture writes");
    }

    let page = LocalStore::new(&database)
        .message_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("the inbox reads");

    assert!(
        page.total >= page.rows.len() as u32 && page.total > 0,
        "the store answered {} rows and a total of {}",
        page.rows.len(),
        page.total
    );
}

async fn page_ids(
    store: &LocalStore,
    mailbox: postio_model::ids::MailboxId,
    offset: u32,
    limit: u32,
) -> Vec<postio_model::ids::MessageId> {
    store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(mailbox),
            offset,
            limit,
        })
        .await
        .expect("the page reads")
        .rows
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[tokio::test]
async fn a_ranked_set_of_ids_reads_back_in_that_order() {
    let (database, report) = seeded().await;
    let store = LocalStore::new(&database);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let page = store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("the inbox reads");
    assert!(
        page.rows.len() >= 4,
        "the seed is too small to rank: {} rows",
        page.rows.len()
    );

    let ranked: Vec<_> = [3usize, 0, 2, 1]
        .iter()
        .map(|at| page.rows[*at].id)
        .collect();

    let rows = store
        .message_rows(ranked.clone())
        .await
        .expect("the hits read");

    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        ranked,
        "the ranking was lost somewhere between the id list and the rows"
    );

    assert!(
        rows.iter().all(|row| row.thread_count >= 1),
        "a hit came back with no thread count, so its badge would vanish"
    );
    assert_eq!(
        rows[0].subject, page.rows[3].subject,
        "the row is not the message that was asked for"
    );

    assert!(
        store
            .message_rows(Vec::new())
            .await
            .expect("empty")
            .is_empty(),
        "an empty id list should not reach SQLite at all"
    );
}

#[tokio::test]
async fn a_thread_reads_across_every_folder_it_touches() {
    use postio_model::ids::ThreadId;
    use postio_storage::repository::MessageRepository;

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    connection
        .execute(
            "INSERT INTO threads (id, account_id) VALUES (1, ?1)",
            [account.id.get()],
        )
        .await
        .expect("a thread");

    let messages = MessageRepository::new(&connection);
    let mut filed = Vec::new();
    for (index, mailbox) in [inbox, archive.id, inbox, archive.id]
        .into_iter()
        .enumerate()
    {
        let mut message = postio_model::Message::new(
            account.id,
            mailbox,
            chrono::Utc::now() - chrono::Duration::minutes(index as i64),
        );
        message.subject = Some("the whole conversation".into());
        let id = messages.create(&mut message).await.expect("create");
        messages
            .set_thread(id, Some(ThreadId::new(1)))
            .await
            .expect("assign");
        filed.push((id, mailbox));
    }
    drop(connection);

    let store = LocalStore::new(&database);
    let page = store
        .message_page(PageRequest {
            scope: ListScope::Thread(ThreadId::new(1)),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("the thread reads");

    assert_eq!(
        page.total, 4,
        "the thread spans two folders and the scope has to span them too"
    );
    let read: std::collections::BTreeSet<_> = page.rows.iter().map(|row| row.id).collect();
    let expected: std::collections::BTreeSet<_> = filed.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        read, expected,
        "every message of the thread, and only those"
    );

    let in_the_inbox = store
        .message_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("the inbox reads");
    assert_eq!(
        in_the_inbox.total, 2,
        "the fixture is wrong if one folder already holds the whole thread"
    );
}
