//! Threads: aggregates, membership, ordering, and the thread list row.
//!
//! Written before the repository existed. The bead's acceptance criteria are
//! "the thread row exposes count and participants without an N+1", "ordering
//! both directions is tested" and "adding a message updates the denormalized
//! fields".


use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    AccountId, EmailAddress, Flag, MailboxId, Message, MessageId, Thread, ThreadId,
};
use postio_storage::repository::{
    MessageRepository, ThreadListQuery, ThreadOrder, ThreadRepository,
};
use postio_storage::test_support;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

/// One message in `mailbox`, from `sender`, received at `seconds`.
async fn message(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    sender: &str,
    seconds: i64,
) -> Message {
    let mut message = Message::new(account, mailbox, at(seconds));
    message.subject = Some(format!("Re: Tide gate interlock {seconds}"));
    message.from = vec![EmailAddress::new(
        Some(sender),
        format!("{sender}@example.com"),
    )];
    message.preview = Some(format!("Snippet {seconds}"));
    message.flags = [Flag::Seen].into_iter().collect();
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create a message");
    message
}

async fn a_thread(connection: &Connection, account: AccountId) -> Thread {
    let mut thread = Thread::new(account);
    thread.subject = Some("tide gate interlock".to_owned());
    ThreadRepository::new(connection)
        .create(&mut thread)
        .await
        .expect("create a thread");
    thread
}

// ---------------------------------------------------------------------------
// Create, read, delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_thread_round_trips_with_its_membership_derived_from_its_messages() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    assert!(thread.id.is_assigned());

    let root = message(&connection, account.id, inbox, "ada", 100).await;
    let reply = message(&connection, account.id, inbox, "quinn", 200).await;
    threads.add_message(thread.id, root.id).await.expect("add");
    threads.add_message(thread.id, reply.id).await.expect("add");

    let stored = threads.get(thread.id).await.expect("get").expect("the thread");
    assert_eq!(
        stored.message_ids,
        vec![root.id, reply.id],
        "members are oldest first, which is Thread::message_ids order"
    );
    assert_eq!(stored.root_message_id(), Some(root.id));
    assert_eq!(stored.latest_message_id(), Some(reply.id));
    assert_eq!(stored.message_count, 2);
    assert_eq!(stored.first_at, at(100));
    assert_eq!(stored.last_at, at(200));
    assert_eq!(stored.mailbox_ids, vec![inbox]);
    assert_eq!(
        stored
            .participants
            .iter()
            .map(|address| address.address.as_str())
            .collect::<Vec<_>>(),
        ["ada@example.com", "quinn@example.com"],
        "participants are in first-seen order"
    );
}

#[tokio::test]
async fn reading_a_thread_that_is_not_there_is_none() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let threads = ThreadRepository::new(&connection);

    assert!(threads.get(ThreadId::new(404)).await.expect("get").is_none());
    assert!(!threads.delete(ThreadId::new(404)).await.expect("delete"));
}

#[tokio::test]
async fn deleting_a_thread_leaves_its_messages_alone() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let message = message(&connection, account.id, inbox, "ada", 10).await;
    threads.add_message(thread.id, message.id).await.expect("add");

    assert!(threads.delete(thread.id).await.expect("delete"));

    let stored = MessageRepository::new(&connection)
        .get(message.id)
        .await
        .expect("get")
        .expect("the message survives");
    assert_eq!(
        stored.thread_id, None,
        "a message outlives the thread it was grouped into; threading can run again"
    );
}

// ---------------------------------------------------------------------------
// Acceptance: adding a message updates the denormalized fields
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adding_a_message_updates_the_threads_aggregates() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let root = message(&connection, account.id, inbox, "ada", 100).await;
    threads.add_message(thread.id, root.id).await.expect("add");

    let after_root = threads.get(thread.id).await.expect("get").expect("the thread");
    assert_eq!(after_root.message_count, 1);
    assert_eq!(after_root.unread_count, 0);
    assert!(!after_root.is_flagged && !after_root.has_attachments);
    assert_eq!(after_root.last_at, at(100));

    let mut reply = Message::new(account.id, inbox, at(300));
    reply.subject = Some("Re: Tide gate interlock".to_owned());
    reply.from = vec![EmailAddress::new(None::<String>, "quinn@example.com")];
    reply.flags = [Flag::Flagged].into_iter().collect();
    reply.attachments = vec![postio_model::Attachment::new(
        MessageId::UNASSIGNED,
        "application/pdf",
        10,
    )];
    messages.create(&mut reply).await.expect("create");
    threads.add_message(thread.id, reply.id).await.expect("add");

    let after_reply = threads.get(thread.id).await.expect("get").expect("the thread");
    assert_eq!(after_reply.message_count, 2);
    assert_eq!(after_reply.unread_count, 1, "the reply is unread");
    assert!(after_reply.has_unread());
    assert!(
        after_reply.is_flagged,
        "one flagged member flags the thread"
    );
    assert!(after_reply.has_attachments);
    assert_eq!(after_reply.last_at, at(300), "the thread moves to the top");
    assert_eq!(after_reply.first_at, at(100), "and keeps its start");
}

#[tokio::test]
async fn the_threads_subject_is_the_normalized_subject_of_its_root() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let mut root = Message::new(account.id, inbox, at(10));
    root.subject = Some("Tide gate interlock".to_owned());
    messages.create(&mut root).await.expect("create");
    let mut reply = Message::new(account.id, inbox, at(20));
    reply.subject = Some("Re: Re: Tide gate interlock".to_owned());
    messages.create(&mut reply).await.expect("create");

    let mut thread = Thread::new(account.id);
    threads.create(&mut thread).await.expect("create");
    threads.add_message(thread.id, reply.id).await.expect("add");
    threads.add_message(thread.id, root.id).await.expect("add");

    assert_eq!(
        threads
            .get(thread.id)
            .await
            .expect("get")
            .expect("the thread")
            .subject
            .as_deref(),
        Some("tide gate interlock"),
        "the oldest member names the conversation, with Re: stripped"
    );
}

#[tokio::test]
async fn removing_a_message_updates_the_aggregates_too() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let root = message(&connection, account.id, inbox, "ada", 100).await;
    let reply = message(&connection, account.id, inbox, "quinn", 200).await;
    threads.add_message(thread.id, root.id).await.expect("add");
    threads.add_message(thread.id, reply.id).await.expect("add");

    threads.remove_message(reply.id).await.expect("remove");

    let stored = threads.get(thread.id).await.expect("get").expect("the thread");
    assert_eq!(stored.message_count, 1);
    assert_eq!(stored.last_at, at(100));
    assert_eq!(
        MessageRepository::new(&connection)
            .get(reply.id)
            .await
            .expect("get")
            .expect("the message")
            .thread_id,
        None
    );
}

#[tokio::test]
async fn a_locally_deleted_message_leaves_the_threads_counts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let root = message(&connection, account.id, inbox, "ada", 100).await;
    let reply = message(&connection, account.id, inbox, "quinn", 200).await;
    threads.add_message(thread.id, root.id).await.expect("add");
    threads.add_message(thread.id, reply.id).await.expect("add");

    messages
        .set_deleted_locally(&[reply.id], true)
        .await
        .expect("hide");
    threads.recompute(thread.id).await.expect("recompute");

    let stored = threads.get(thread.id).await.expect("get").expect("the thread");
    assert_eq!(
        stored.message_count, 1,
        "the list hides it, so it is not counted"
    );
    assert_eq!(stored.last_at, at(100));
    assert_eq!(
        stored.message_ids,
        vec![root.id],
        "and it is not in the drill-in either"
    );
}

// ---------------------------------------------------------------------------
// Acceptance: ordering in both directions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_threads_messages_can_be_read_oldest_or_newest_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let first = message(&connection, account.id, inbox, "ada", 100).await;
    let second = message(&connection, account.id, inbox, "quinn", 200).await;
    let third = message(&connection, account.id, inbox, "tove", 300).await;
    for id in [first.id, second.id, third.id] {
        threads.add_message(thread.id, id).await.expect("add");
    }

    let oldest: Vec<MessageId> = threads
        .messages(thread.id, ThreadOrder::Oldest)
        .await
        .expect("oldest first")
        .iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(
        oldest,
        [first.id, second.id, third.id],
        "reading a thread runs down the page in the order it happened"
    );

    let newest: Vec<MessageId> = threads
        .messages(thread.id, ThreadOrder::Newest)
        .await
        .expect("newest first")
        .iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(newest, [third.id, second.id, first.id]);
}

#[tokio::test]
async fn reading_a_thread_in_either_direction_never_sorts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let threads = ThreadRepository::new(&connection);

    for order in [ThreadOrder::Oldest, ThreadOrder::Newest] {
        let sql = threads.explain_messages(order);
        let plan = test_support::plan(&connection, &sql).await;

        assert!(
            !plan.contains("TEMP B-TREE"),
            "{order:?}: the drill-in must not sort:\n{plan}"
        );
        assert!(
            plan.contains("idx_messages_thread"),
            "{order:?}: expected the thread index:\n{plan}"
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance: the list row, count and participants, without an N+1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_page_of_threads_costs_a_fixed_number_of_queries() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    // Twenty threads of three messages each, from three different senders.
    for index in 0..20 {
        let thread = a_thread(&connection, account.id).await;
        for reply in 0..3 {
            let sender = ["ada", "quinn", "tove"][reply as usize];
            let message = message(
                &connection,
                account.id,
                inbox,
                sender,
                index * 1_000 + reply * 10,
            ).await;
            ThreadRepository::new(&connection)
                .add_message(thread.id, message.id)
                .await
                .expect("add");
        }
    }

    postio_storage::test_support::counting::reset();
    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::account(account.id).limit(20))
        .await
        .expect("page");
    let statements = postio_storage::test_support::counting::here().statements;

    assert_eq!(page.len(), 20);
    assert!(
        statements <= 4,
        "a page of 20 threads with their participants and latest message took \
         {statements} statements; that has to be a constant, not one per row"
    );

    let row = &page[0];
    assert_eq!(row.message_count, 3);
    assert_eq!(
        row.participants
            .iter()
            .map(|address| address.address.as_str())
            .collect::<Vec<_>>(),
        ["ada@example.com", "quinn@example.com", "tove@example.com"],
        "every participant, deduplicated, in first-seen order"
    );
    let latest = row
        .latest
        .as_ref()
        .expect("the newest message in the thread");
    assert_eq!(latest.received_at, row.last_at);
    assert_eq!(
        latest.from.as_ref().map(|from| from.address.as_str()),
        Some("tove@example.com"),
        "the row shows the newest message's sender and snippet"
    );
    assert!(latest.preview.is_some());
}

#[tokio::test]
async fn the_thread_list_is_newest_first_and_pages_by_cursor() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    for index in 0..25 {
        let thread = a_thread(&connection, account.id).await;
        let message = message(&connection, account.id, inbox, "ada", index * 100).await;
        threads.add_message(thread.id, message.id).await.expect("add");
    }

    let first = threads
        .page(&ThreadListQuery::account(account.id).limit(10))
        .await
        .expect("page");
    assert_eq!(first.len(), 10);
    assert!(
        first[0].last_at > first[1].last_at,
        "the most recently active conversation is at the top"
    );

    let mut seen: Vec<Option<ThreadId>> = first.iter().map(|row| row.id).collect();
    let mut cursor = first.last().expect("a row").cursor();
    loop {
        let page = threads
            .page(&ThreadListQuery::account(account.id).limit(10).after(cursor))
            .await
            .expect("page");
        let Some(last) = page.last() else { break };
        cursor = last.cursor();
        seen.extend(page.iter().map(|row| row.id));
    }

    assert_eq!(seen.len(), 25);
    seen.dedup();
    assert_eq!(seen.len(), 25, "no thread appears twice");
    assert_eq!(threads.count(account.id).await.expect("count"), 25);
}

#[tokio::test]
async fn a_thread_whose_messages_are_all_hidden_drops_out_of_the_list() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let only = message(&connection, account.id, inbox, "ada", 10).await;
    threads.add_message(thread.id, only.id).await.expect("add");
    assert_eq!(
        threads
            .page(&ThreadListQuery::account(account.id))
            .await
            .expect("page")
            .len(),
        1
    );

    messages
        .set_deleted_locally(&[only.id], true)
        .await
        .expect("hide");
    threads.recompute(thread.id).await.expect("recompute");

    assert!(
        threads
            .page(&ThreadListQuery::account(account.id))
            .await
            .expect("page")
            .is_empty(),
        "an empty conversation is not a row the user can do anything with"
    );
    assert!(
        threads.get(thread.id).await.expect("get").is_some(),
        "but it is still there for undo to restore"
    );
}

#[tokio::test]
async fn the_thread_list_plan_never_sorts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let threads = ThreadRepository::new(&connection);

    for (label, base) in [
        ("account", ThreadListQuery::account(AccountId::new(1))),
        (
            "mailbox",
            ThreadListQuery::in_mailbox(AccountId::new(1), MailboxId::new(1)),
        ),
    ] {
        for after in [false, true] {
            let mut query = base.clone();
            if after {
                query = query.after(postio_storage::repository::ThreadCursor {
                    last_at: at(0),
                    id: 10,
                });
            }
            let sql = threads.explain(&query);
            let plan = test_support::plan(&connection, &sql).await;

            assert!(
                !plan.contains("TEMP B-TREE"),
                "{label} / cursor={after}: the thread list must never sort:\n{plan}"
            );
            assert!(
                !plan.contains("SCAN threads") && !plan.contains("SCAN messages"),
                "{label} / cursor={after}: the list must never scan a table:\n{plan}"
            );
            if base.mailbox.is_some() {
                // The folder window is ordered over the *representative
                // message*, on the very index the message list uses — which
                // is what makes "page k of threads costs what page k of
                // messages costs" true by construction rather than by
                // measurement.
                assert!(
                    plan.contains("idx_messages_list"),
                    "{label} / cursor={after}: the folder window must walk \
                     the message list index:\n{plan}"
                );
                // Everything the conversation contributes is a correlated
                // subquery, and each has to seek the index migration 0012
                // added rather than walk a whole thread and filter.
                assert!(
                    plan.contains("idx_messages_thread_mailbox"),
                    "{label} / cursor={after}: the folder slice must seek its \
                     own index:\n{plan}"
                );
            } else {
                assert!(
                    plan.contains("idx_threads_account_last_at"),
                    "{label} / cursor={after}: expected the thread list \
                     index:\n{plan}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Merge, for a late message that links two roots
// ---------------------------------------------------------------------------

#[tokio::test]
async fn merging_moves_every_message_and_leaves_one_thread() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    let keep = a_thread(&connection, account.id).await;
    let absorb = a_thread(&connection, account.id).await;
    let older = message(&connection, account.id, inbox, "ada", 100).await;
    let newer = message(&connection, account.id, inbox, "quinn", 500).await;
    threads.add_message(keep.id, older.id).await.expect("add");
    threads.add_message(absorb.id, newer.id).await.expect("add");

    threads.merge(keep.id, absorb.id).await.expect("merge");

    let merged = threads.get(keep.id).await.expect("get").expect("the thread");
    assert_eq!(merged.message_ids, vec![older.id, newer.id]);
    assert_eq!(merged.message_count, 2);
    assert_eq!(merged.last_at, at(500), "the aggregates were recomputed");
    assert!(
        threads.get(absorb.id).await.expect("get").is_none(),
        "the absorbed thread is gone, not left empty"
    );
}

#[tokio::test]
async fn merging_a_thread_into_itself_does_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id).await;
    let only = message(&connection, account.id, inbox, "ada", 10).await;
    threads.add_message(thread.id, only.id).await.expect("add");

    threads.merge(thread.id, thread.id).await.expect("merge");

    let stored = threads.get(thread.id).await.expect("get").expect("still there");
    assert_eq!(stored.message_count, 1);
}

// ---------------------------------------------------------------------------
// The folder-scoped thread list (ADR 0015, #306)
// ---------------------------------------------------------------------------
//
// A folder shows one row per conversation. The window is over `threads` and
// never over messages that are then collapsed — reading rows to throw them
// away is what breaks flat paging and the windowed-list invariant.
//
// Three things about a row are scoped to the folder rather than to the whole
// conversation: whether the thread appears at all, which message represents
// it, and how much of it is unread *here*. The total message count is not —
// the badge means the size of the conversation (ADR 0015 Q2).

/// A message in `mailbox`, unread, threaded into `thread`.
async fn unread_in(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    thread: ThreadId,
    seconds: i64,
) -> MessageId {
    let mut message = Message::new(account, mailbox, at(seconds));
    message.subject = Some("Tide gate interlock".to_owned());
    message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
    let id = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create a message");
    let threads = ThreadRepository::new(connection);
    threads.add_message(thread, id).await.expect("thread it");
    id
}

#[tokio::test]
async fn a_folder_only_shows_conversations_it_holds_a_message_of() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await.id;

    let here = a_thread(&connection, account.id).await;
    let elsewhere = a_thread(&connection, account.id).await;
    unread_in(&connection, account.id, inbox, here.id, 10).await;
    unread_in(&connection, account.id, archive, elsewhere.id, 20).await;

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page of threads");

    let ids: Vec<Option<ThreadId>> = page.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        vec![Some(here.id)],
        "a conversation with nothing filed in this folder is not this \
         folder's row, even though it is newer"
    );
}

#[tokio::test]
async fn the_row_is_drawn_from_the_newest_message_in_this_folder() {
    // The representative, and the reason it is scoped: a reply filed in
    // Archive is not what the Inbox row should be showing.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await.id;

    let thread = a_thread(&connection, account.id).await;
    let in_inbox = unread_in(&connection, account.id, inbox, thread.id, 10).await;
    let newest_overall = unread_in(&connection, account.id, archive, thread.id, 99).await;

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page of threads");

    let latest = page[0].latest.as_ref().expect("a representative message");
    assert_eq!(latest.id, in_inbox);
    assert_ne!(
        latest.id, newest_overall,
        "the representative is the newest message *here*, not the newest \
         message in the conversation"
    );
}

#[tokio::test]
async fn unread_is_counted_in_this_folder_and_the_total_is_not() {
    // ADR 0015 Q2, exactly: a thread whose only unread member is in another
    // folder reads as handled in this one — but the badge still says how big
    // the conversation is.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await.id;

    let thread = a_thread(&connection, account.id).await;
    // Two unread here, three unread over there.
    unread_in(&connection, account.id, inbox, thread.id, 10).await;
    unread_in(&connection, account.id, inbox, thread.id, 11).await;
    for second in 20..23 {
        unread_in(&connection, account.id, archive, thread.id, second).await;
    }

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page of threads");

    assert_eq!(page[0].unread_count, 2, "unread is this folder's slice");
    assert_eq!(
        page[0].message_count, 5,
        "the count is the size of the conversation, wherever it is filed"
    );
}

#[tokio::test]
async fn a_folder_with_only_read_messages_of_a_thread_reads_as_handled() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await.id;

    let thread = a_thread(&connection, account.id).await;
    // `message` marks Seen; `unread_in` does not.
    let read = message(&connection, account.id, inbox, "ada", 10).await;
    ThreadRepository::new(&connection)
        .add_message(thread.id, read.id)
        .await
        .expect("thread it");
    unread_in(&connection, account.id, archive, thread.id, 20).await;

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page of threads");

    assert!(
        !page[0].has_unread(),
        "the unread member is in another folder, so there is nothing to do \
         about this conversation here"
    );
}

#[tokio::test]
async fn the_account_scoped_list_is_unchanged_by_any_of_this() {
    // Query views still list messages and the unified thread list still spans
    // folders; scoping is something a folder asks for, not the new default.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await.id;

    let thread = a_thread(&connection, account.id).await;
    unread_in(&connection, account.id, inbox, thread.id, 10).await;
    let newest = unread_in(&connection, account.id, archive, thread.id, 99).await;

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::account(account.id))
        .await
        .expect("a page of threads");

    assert_eq!(page[0].unread_count, 2);
    assert_eq!(
        page[0].latest.as_ref().expect("a newest message").id,
        newest,
        "unscoped, the newest message in the conversation represents it"
    );
}

#[tokio::test]
async fn a_folder_scoped_thread_page_resumes_after_its_cursor() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    // A loop rather than `map().collect()`: the body awaits, and a
    // closure cannot.
    let mut threads: Vec<Thread> = Vec::new();
    for _ in 0..5 {
        threads.push(a_thread(&connection, account.id).await);
    }
    for (index, thread) in threads.iter().enumerate() {
        unread_in(&connection, account.id, inbox, thread.id, index as i64 * 10).await;
    }

    let repository = ThreadRepository::new(&connection);
    let first = repository
        .page(&ThreadListQuery::in_mailbox(account.id, inbox).limit(2))
        .await
        .expect("the first page");
    assert_eq!(first.len(), 2);

    let second = repository
        .page(
            &ThreadListQuery::in_mailbox(account.id, inbox)
                .limit(2)
                .after(first[1].cursor()),
        )
        .await
        .expect("the second page");

    assert_eq!(second.len(), 2);
    assert!(
        second[0].last_at <= first[1].last_at,
        "paging walks strictly backwards through the sort key"
    );
    let seen: Vec<Option<ThreadId>> = first
        .iter()
        .chain(second.iter())
        .map(|row| row.id)
        .collect();
    let mut unique = seen.clone();
    unique.sort_by_key(|id| id.map(|id| id.get()).unwrap_or_default());
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "no row is paged twice");
}

#[tokio::test]
async fn a_hidden_message_is_not_what_a_folder_row_shows() {
    // A message hidden pending a remote delete is not a member anywhere else
    // either; the scoped representative has to agree.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let thread = a_thread(&connection, account.id).await;
    let visible = unread_in(&connection, account.id, inbox, thread.id, 10).await;
    let hidden = unread_in(&connection, account.id, inbox, thread.id, 99).await;
    connection
        .execute(
            "UPDATE messages SET deleted_locally = 1 WHERE id = ?1",
            [hidden.get()],
        )
        .await
        .expect("hide it");

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page of threads");

    assert_eq!(
        page[0].latest.as_ref().expect("a representative").id,
        visible
    );
    assert_eq!(page[0].unread_count, 1);
}

#[tokio::test]
async fn thread_paging_stays_flat_over_a_hundred_thousand_messages() {
    // The claim ADR 0015 rests on: page k of *threads* costs what page k of
    // messages costs. If the folder scoping had turned the window into
    // something linear in the size of the mailbox, this is where it shows —
    // the correlated subqueries are per row of the page, so twenty-five
    // thousand conversations must not cost more than a handful.
    //
    // Counted rather than timed (#100). This used to compare wall-clock
    // durations and allow the deep page a factor of ten plus 50ms, which is
    // the width a shared runner forces on a timing assertion — wide enough
    // that it would not have caught much short of the linear case it names.
    // Rows produced is the same number on any machine, so it can be held to
    // the actual claim: a page of fifty costs a page of fifty, ten pages in.
    let database = postio_storage::test_support::temp().await;
    let report = postio_storage::seed::seed_large(&database, 7, 100_000).await;
    let inbox = report
        .mailbox(postio_model::mailbox::MailboxRole::Inbox)
        .expect("an inbox")
        .id;
    postio_storage::seed::thread_seeded_messages(&database, report.account.id, 4).await;

    let connection = database.connect().await.expect("checkout");
    let threads = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(report.account.id, inbox).limit(50);

    postio_storage::test_support::counting::install(&connection);
    let read = async |query: &ThreadListQuery| {
        let mut page = Vec::new();
        let counts = postio_storage::test_support::counting::counted_async(|| async {
            page = threads.page(query).await.expect("a page of threads");
        })
        .await;
        (counts, page)
    };

    let (first, first_page) = read(&query).await;
    assert_eq!(first_page.len(), 50, "a page is a window, never the folder");

    // Ten pages in, which for messages is the same cost as the first.
    let mut cursor = first_page.last().expect("a last row").cursor();
    let mut deep = first;
    for _ in 0..10 {
        let (counts, page) = read(&query.clone().after(cursor)).await;
        assert_eq!(page.len(), 50);
        cursor = page.last().expect("a last row").cursor();
        deep = counts;
    }

    // Not exact equality: the correlated subqueries return a row per message
    // in each thread on the page, so the count moves a little with the shape
    // of the conversations that happen to be there — 246 and 252 as this
    // landed. Doubling is far more slack than that needs and still leaves
    // this two orders of magnitude away from the linear case it rules out.
    assert!(
        deep.rows <= first.rows * 2,
        "the eleventh page of threads produced {} rows where the first \
         produced {}, over a folder of a hundred thousand messages. Paging \
         that grows with the depth is what ADR 0015's claim rules out, and at \
         the bottom of a real mailbox it is the whole folder.",
        deep.rows,
        first.rows
    );
    assert_eq!(
        deep.statements, first.statements,
        "the eleventh page issued {} statements where the first issued {}, \
         which is a query per row of the page rather than a window.",
        deep.statements, first.statements
    );
}

#[tokio::test]
async fn a_folder_scoped_count_agrees_with_the_rows_it_would_show() {
    // The count and the page must mean the same thing by "in this folder", or
    // the list model's scrollbar promises rows the window cannot produce.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await.id;

    for index in 0..4 {
        let thread = a_thread(&connection, account.id).await;
        unread_in(&connection, account.id, inbox, thread.id, index * 10).await;
    }
    for index in 0..3 {
        let thread = a_thread(&connection, account.id).await;
        unread_in(
            &connection,
            account.id,
            archive,
            thread.id,
            100 + index * 10,
        ).await;
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox);
    assert_eq!(repository.count_of(&query).await.expect("a count"), 4);
    assert_eq!(repository.page(&query).await.expect("a page").len(), 4);
    assert_eq!(
        repository.count(account.id).await.expect("an account count"),
        7,
        "the account still sees every conversation"
    );
}

#[tokio::test]
async fn a_thread_page_at_an_offset_resumes_where_the_previous_one_stopped() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    // A loop rather than `map().collect()`: the body awaits, and a
    // closure cannot.
    let mut threads: Vec<Thread> = Vec::new();
    for _ in 0..6 {
        threads.push(a_thread(&connection, account.id).await);
    }
    for (index, thread) in threads.iter().enumerate() {
        unread_in(&connection, account.id, inbox, thread.id, index as i64 * 10).await;
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox).limit(2);
    let all: Vec<Option<ThreadId>> = repository
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("every row")
        .iter()
        .map(|row| row.id)
        .collect();

    let second: Vec<Option<ThreadId>> = repository
        .page_at(&query, 2)
        .await
        .expect("an offset page")
        .iter()
        .map(|row| row.id)
        .collect();

    assert_eq!(
        second,
        all[2..4],
        "an offset window is the same window, moved"
    );
}

#[tokio::test]
async fn a_message_belonging_to_no_thread_is_still_a_row() {
    // The list must never hide mail. Threading runs on everything sync files
    // and can still fail — `postio-sync`'s send path discards the result with
    // `let _ =` — and a window built only over `threads` makes every such
    // message *invisible*: no error, no empty state, just mail that is in the
    // store and not on screen.
    //
    // So the folder window is over the representative *message* rather than
    // over `threads`, and a message with no thread is a conversation of one.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let threaded = a_thread(&connection, account.id).await;
    unread_in(&connection, account.id, inbox, threaded.id, 10).await;

    // Straight into the table, exactly as a message that was never threaded
    // would sit there.
    let mut orphan = Message::new(account.id, inbox, at(20));
    orphan.subject = Some("Never threaded".to_owned());
    orphan.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    let orphan = MessageRepository::new(&connection)
        .create(&mut orphan)
        .await
        .expect("create an unthreaded message");

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page");

    assert_eq!(page.len(), 2, "the unthreaded message must still be a row");
    let row = page
        .iter()
        .find(|row| {
            row.latest
                .as_ref()
                .is_some_and(|latest| latest.id == orphan)
        })
        .expect("the unthreaded message is one of the rows");
    assert_eq!(row.id, None, "it belongs to no conversation, and says so");
    assert_eq!(row.message_count, 1, "a conversation of one");
    assert_eq!(
        row.participants.len(),
        1,
        "it still has a sender to draw, so the row is not blank"
    );
}

#[tokio::test]
async fn two_unthreaded_messages_are_two_rows_rather_than_one() {
    // The coalesce that lets a null thread id be compared must not make every
    // unthreaded message look like a member of the same conversation.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    for second in [10, 20, 30] {
        let mut message = Message::new(account.id, inbox, at(second));
        message.subject = Some(format!("Never threaded {second}"));
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create an unthreaded message");
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox);
    assert_eq!(repository.page(&query).await.expect("a page").len(), 3);
    assert_eq!(repository.count_of(&query).await.expect("a count"), 3);
}
