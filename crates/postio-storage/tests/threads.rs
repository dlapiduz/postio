//! Threads: aggregates, membership, ordering, and the thread list row.
//!
//! Written before the repository existed. The bead's acceptance criteria are
//! "the thread row exposes count and participants without an N+1", "ordering
//! both directions is tested" and "adding a message updates the denormalized
//! fields".

use std::cell::Cell;

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;

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
fn message(
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
        .expect("create a message");
    message
}

fn a_thread(connection: &Connection, account: AccountId) -> Thread {
    let mut thread = Thread::new(account);
    thread.subject = Some("tide gate interlock".to_owned());
    ThreadRepository::new(connection)
        .create(&mut thread)
        .expect("create a thread");
    thread
}

// ---------------------------------------------------------------------------
// Create, read, delete
// ---------------------------------------------------------------------------

#[test]
fn a_thread_round_trips_with_its_membership_derived_from_its_messages() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    assert!(thread.id.is_assigned());

    let root = message(&connection, account.id, inbox, "ada", 100);
    let reply = message(&connection, account.id, inbox, "quinn", 200);
    threads.add_message(thread.id, root.id).expect("add");
    threads.add_message(thread.id, reply.id).expect("add");

    let stored = threads.get(thread.id).expect("get").expect("the thread");
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

#[test]
fn reading_a_thread_that_is_not_there_is_none() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let threads = ThreadRepository::new(&connection);

    assert!(threads.get(ThreadId::new(404)).expect("get").is_none());
    assert!(!threads.delete(ThreadId::new(404)).expect("delete"));
}

#[test]
fn deleting_a_thread_leaves_its_messages_alone() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let message = message(&connection, account.id, inbox, "ada", 10);
    threads.add_message(thread.id, message.id).expect("add");

    assert!(threads.delete(thread.id).expect("delete"));

    let stored = MessageRepository::new(&connection)
        .get(message.id)
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

#[test]
fn adding_a_message_updates_the_threads_aggregates() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let root = message(&connection, account.id, inbox, "ada", 100);
    threads.add_message(thread.id, root.id).expect("add");

    let after_root = threads.get(thread.id).expect("get").expect("the thread");
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
    messages.create(&mut reply).expect("create");
    threads.add_message(thread.id, reply.id).expect("add");

    let after_reply = threads.get(thread.id).expect("get").expect("the thread");
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

#[test]
fn the_threads_subject_is_the_normalized_subject_of_its_root() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let mut root = Message::new(account.id, inbox, at(10));
    root.subject = Some("Tide gate interlock".to_owned());
    messages.create(&mut root).expect("create");
    let mut reply = Message::new(account.id, inbox, at(20));
    reply.subject = Some("Re: Re: Tide gate interlock".to_owned());
    messages.create(&mut reply).expect("create");

    let mut thread = Thread::new(account.id);
    threads.create(&mut thread).expect("create");
    threads.add_message(thread.id, reply.id).expect("add");
    threads.add_message(thread.id, root.id).expect("add");

    assert_eq!(
        threads
            .get(thread.id)
            .expect("get")
            .expect("the thread")
            .subject
            .as_deref(),
        Some("tide gate interlock"),
        "the oldest member names the conversation, with Re: stripped"
    );
}

#[test]
fn removing_a_message_updates_the_aggregates_too() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let root = message(&connection, account.id, inbox, "ada", 100);
    let reply = message(&connection, account.id, inbox, "quinn", 200);
    threads.add_message(thread.id, root.id).expect("add");
    threads.add_message(thread.id, reply.id).expect("add");

    threads.remove_message(reply.id).expect("remove");

    let stored = threads.get(thread.id).expect("get").expect("the thread");
    assert_eq!(stored.message_count, 1);
    assert_eq!(stored.last_at, at(100));
    assert_eq!(
        MessageRepository::new(&connection)
            .get(reply.id)
            .expect("get")
            .expect("the message")
            .thread_id,
        None
    );
}

#[test]
fn a_locally_deleted_message_leaves_the_threads_counts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let root = message(&connection, account.id, inbox, "ada", 100);
    let reply = message(&connection, account.id, inbox, "quinn", 200);
    threads.add_message(thread.id, root.id).expect("add");
    threads.add_message(thread.id, reply.id).expect("add");

    messages
        .set_deleted_locally(&[reply.id], true)
        .expect("hide");
    threads.recompute(thread.id).expect("recompute");

    let stored = threads.get(thread.id).expect("get").expect("the thread");
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

#[test]
fn a_threads_messages_can_be_read_oldest_or_newest_first() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let first = message(&connection, account.id, inbox, "ada", 100);
    let second = message(&connection, account.id, inbox, "quinn", 200);
    let third = message(&connection, account.id, inbox, "tove", 300);
    for id in [first.id, second.id, third.id] {
        threads.add_message(thread.id, id).expect("add");
    }

    let oldest: Vec<MessageId> = threads
        .messages(thread.id, ThreadOrder::Oldest)
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
        .expect("newest first")
        .iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(newest, [third.id, second.id, first.id]);
}

#[test]
fn reading_a_thread_in_either_direction_never_sorts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let threads = ThreadRepository::new(&connection);

    for order in [ThreadOrder::Oldest, ThreadOrder::Newest] {
        let sql = threads.explain_messages(order);
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .expect("prepare");
        let arguments = vec![1i64; statement.parameter_count()];
        let plan = statement
            .query_map(rusqlite::params_from_iter(arguments), |row| {
                row.get::<_, String>(3)
            })
            .expect("plan")
            .collect::<Result<Vec<String>, _>>()
            .expect("collect")
            .join("\n");

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

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
}

/// Counts every statement SQLite starts running.
///
/// A plain `fn`, not a closure: that is what `trace_v2` takes, so the counter
/// has to live outside it.
fn count_statement(event: rusqlite::trace::TraceEvent<'_>) {
    if matches!(event, rusqlite::trace::TraceEvent::Stmt(..)) {
        STATEMENTS.with(|count| count.set(count.get() + 1));
    }
}

#[test]
fn a_page_of_threads_costs_a_fixed_number_of_queries() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // Twenty threads of three messages each, from three different senders.
    for index in 0..20 {
        let thread = a_thread(&connection, account.id);
        for reply in 0..3 {
            let sender = ["ada", "quinn", "tove"][reply as usize];
            let message = message(
                &connection,
                account.id,
                inbox,
                sender,
                index * 1_000 + reply * 10,
            );
            ThreadRepository::new(&connection)
                .add_message(thread.id, message.id)
                .expect("add");
        }
    }

    STATEMENTS.with(|count| count.set(0));
    connection.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
        Some(count_statement),
    );
    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::account(account.id).limit(20))
        .expect("page");
    connection.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    let statements = STATEMENTS.with(Cell::get);

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

#[test]
fn the_thread_list_is_newest_first_and_pages_by_cursor() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    for index in 0..25 {
        let thread = a_thread(&connection, account.id);
        let message = message(&connection, account.id, inbox, "ada", index * 100);
        threads.add_message(thread.id, message.id).expect("add");
    }

    let first = threads
        .page(&ThreadListQuery::account(account.id).limit(10))
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
            .expect("page");
        let Some(last) = page.last() else { break };
        cursor = last.cursor();
        seen.extend(page.iter().map(|row| row.id));
    }

    assert_eq!(seen.len(), 25);
    seen.dedup();
    assert_eq!(seen.len(), 25, "no thread appears twice");
    assert_eq!(threads.count(account.id).expect("count"), 25);
}

#[test]
fn a_thread_whose_messages_are_all_hidden_drops_out_of_the_list() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let only = message(&connection, account.id, inbox, "ada", 10);
    threads.add_message(thread.id, only.id).expect("add");
    assert_eq!(
        threads
            .page(&ThreadListQuery::account(account.id))
            .expect("page")
            .len(),
        1
    );

    messages
        .set_deleted_locally(&[only.id], true)
        .expect("hide");
    threads.recompute(thread.id).expect("recompute");

    assert!(
        threads
            .page(&ThreadListQuery::account(account.id))
            .expect("page")
            .is_empty(),
        "an empty conversation is not a row the user can do anything with"
    );
    assert!(
        threads.get(thread.id).expect("get").is_some(),
        "but it is still there for undo to restore"
    );
}

#[test]
fn the_thread_list_plan_never_sorts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
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
            let mut statement = connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .expect("prepare");
            let arguments = vec![1i64; statement.parameter_count()];
            let plan = statement
                .query_map(rusqlite::params_from_iter(arguments), |row| {
                    row.get::<_, String>(3)
                })
                .expect("plan")
                .collect::<Result<Vec<String>, _>>()
                .expect("collect")
                .join("\n");

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

#[test]
fn merging_moves_every_message_and_leaves_one_thread() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    let keep = a_thread(&connection, account.id);
    let absorb = a_thread(&connection, account.id);
    let older = message(&connection, account.id, inbox, "ada", 100);
    let newer = message(&connection, account.id, inbox, "quinn", 500);
    threads.add_message(keep.id, older.id).expect("add");
    threads.add_message(absorb.id, newer.id).expect("add");

    threads.merge(keep.id, absorb.id).expect("merge");

    let merged = threads.get(keep.id).expect("get").expect("the thread");
    assert_eq!(merged.message_ids, vec![older.id, newer.id]);
    assert_eq!(merged.message_count, 2);
    assert_eq!(merged.last_at, at(500), "the aggregates were recomputed");
    assert!(
        threads.get(absorb.id).expect("get").is_none(),
        "the absorbed thread is gone, not left empty"
    );
}

#[test]
fn merging_a_thread_into_itself_does_nothing() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threads = ThreadRepository::new(&connection);

    let thread = a_thread(&connection, account.id);
    let only = message(&connection, account.id, inbox, "ada", 10);
    threads.add_message(thread.id, only.id).expect("add");

    threads.merge(thread.id, thread.id).expect("merge");

    let stored = threads.get(thread.id).expect("get").expect("still there");
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
fn unread_in(
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
        .expect("create a message");
    let threads = ThreadRepository::new(connection);
    threads.add_message(thread, id).expect("thread it");
    id
}

#[test]
fn a_folder_only_shows_conversations_it_holds_a_message_of() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let here = a_thread(&connection, account.id);
    let elsewhere = a_thread(&connection, account.id);
    unread_in(&connection, account.id, inbox, here.id, 10);
    unread_in(&connection, account.id, archive, elsewhere.id, 20);

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .expect("a page of threads");

    let ids: Vec<Option<ThreadId>> = page.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        vec![Some(here.id)],
        "a conversation with nothing filed in this folder is not this \
         folder's row, even though it is newer"
    );
}

#[test]
fn the_row_is_drawn_from_the_newest_message_in_this_folder() {
    // The representative, and the reason it is scoped: a reply filed in
    // Archive is not what the Inbox row should be showing.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let thread = a_thread(&connection, account.id);
    let in_inbox = unread_in(&connection, account.id, inbox, thread.id, 10);
    let newest_overall = unread_in(&connection, account.id, archive, thread.id, 99);

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .expect("a page of threads");

    let latest = page[0].latest.as_ref().expect("a representative message");
    assert_eq!(latest.id, in_inbox);
    assert_ne!(
        latest.id, newest_overall,
        "the representative is the newest message *here*, not the newest \
         message in the conversation"
    );
}

#[test]
fn unread_is_counted_in_this_folder_and_the_total_is_not() {
    // ADR 0015 Q2, exactly: a thread whose only unread member is in another
    // folder reads as handled in this one — but the badge still says how big
    // the conversation is.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let thread = a_thread(&connection, account.id);
    // Two unread here, three unread over there.
    unread_in(&connection, account.id, inbox, thread.id, 10);
    unread_in(&connection, account.id, inbox, thread.id, 11);
    for second in 20..23 {
        unread_in(&connection, account.id, archive, thread.id, second);
    }

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .expect("a page of threads");

    assert_eq!(page[0].unread_count, 2, "unread is this folder's slice");
    assert_eq!(
        page[0].message_count, 5,
        "the count is the size of the conversation, wherever it is filed"
    );
}

#[test]
fn a_folder_with_only_read_messages_of_a_thread_reads_as_handled() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let thread = a_thread(&connection, account.id);
    // `message` marks Seen; `unread_in` does not.
    let read = message(&connection, account.id, inbox, "ada", 10);
    ThreadRepository::new(&connection)
        .add_message(thread.id, read.id)
        .expect("thread it");
    unread_in(&connection, account.id, archive, thread.id, 20);

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .expect("a page of threads");

    assert!(
        !page[0].has_unread(),
        "the unread member is in another folder, so there is nothing to do \
         about this conversation here"
    );
}

#[test]
fn the_account_scoped_list_is_unchanged_by_any_of_this() {
    // Query views still list messages and the unified thread list still spans
    // folders; scoping is something a folder asks for, not the new default.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    let thread = a_thread(&connection, account.id);
    unread_in(&connection, account.id, inbox, thread.id, 10);
    let newest = unread_in(&connection, account.id, archive, thread.id, 99);

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::account(account.id))
        .expect("a page of threads");

    assert_eq!(page[0].unread_count, 2);
    assert_eq!(
        page[0].latest.as_ref().expect("a newest message").id,
        newest,
        "unscoped, the newest message in the conversation represents it"
    );
}

#[test]
fn a_folder_scoped_thread_page_resumes_after_its_cursor() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let threads: Vec<Thread> = (0..5).map(|_| a_thread(&connection, account.id)).collect();
    for (index, thread) in threads.iter().enumerate() {
        unread_in(&connection, account.id, inbox, thread.id, index as i64 * 10);
    }

    let repository = ThreadRepository::new(&connection);
    let first = repository
        .page(&ThreadListQuery::in_mailbox(account.id, inbox).limit(2))
        .expect("the first page");
    assert_eq!(first.len(), 2);

    let second = repository
        .page(
            &ThreadListQuery::in_mailbox(account.id, inbox)
                .limit(2)
                .after(first[1].cursor()),
        )
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

#[test]
fn a_hidden_message_is_not_what_a_folder_row_shows() {
    // A message hidden pending a remote delete is not a member anywhere else
    // either; the scoped representative has to agree.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let thread = a_thread(&connection, account.id);
    let visible = unread_in(&connection, account.id, inbox, thread.id, 10);
    let hidden = unread_in(&connection, account.id, inbox, thread.id, 99);
    connection
        .execute(
            "UPDATE messages SET deleted_locally = 1 WHERE id = ?1",
            [hidden.get()],
        )
        .expect("hide it");

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .expect("a page of threads");

    assert_eq!(
        page[0].latest.as_ref().expect("a representative").id,
        visible
    );
    assert_eq!(page[0].unread_count, 1);
}

#[test]
fn thread_paging_stays_flat_over_a_hundred_thousand_messages() {
    // The claim ADR 0015 rests on: page k of *threads* costs what page k of
    // messages costs. If the folder scoping had turned the window into
    // something linear in the size of the mailbox, this is where it shows —
    // the correlated subqueries are per row of the page, so twenty-five
    // thousand conversations must not cost more than a handful.
    use std::time::Instant;

    let database = postio_storage::test_support::temp();
    let report = postio_storage::seed::seed_large(&database, 7, 100_000);
    let inbox = report
        .mailbox(postio_model::mailbox::MailboxRole::Inbox)
        .expect("an inbox")
        .id;
    postio_storage::seed::thread_seeded_messages(&database, report.account.id, 4);

    let connection = database.connection().expect("checkout");
    let threads = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(report.account.id, inbox).limit(50);

    let time = |query: &ThreadListQuery| {
        let start = Instant::now();
        let page = threads.page(query).expect("a page of threads");
        (start.elapsed(), page)
    };

    let (first_duration, first) = time(&query);
    assert_eq!(first.len(), 50, "a page is a window, never the folder");

    // Ten pages in, which for messages is the same cost as the first.
    let mut cursor = first.last().expect("a last row").cursor();
    let mut deep_duration = first_duration;
    for _ in 0..10 {
        let (duration, page) = time(&query.clone().after(cursor));
        assert_eq!(page.len(), 50);
        cursor = page.last().expect("a last row").cursor();
        deep_duration = duration;
    }

    // Generous, because this is wall-clock on a shared machine: the point is
    // that paging is not *linear*, and linear here would be orders of
    // magnitude rather than a factor of ten.
    assert!(
        deep_duration < first_duration * 10 + std::time::Duration::from_millis(50),
        "a deep page of threads cost {deep_duration:?} against {first_duration:?} \
         for the first, which is paging that grows with the folder"
    );
}

#[test]
fn a_folder_scoped_count_agrees_with_the_rows_it_would_show() {
    // The count and the page must mean the same thing by "in this folder", or
    // the list model's scrollbar promises rows the window cannot produce.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let archive = test_support::mailbox(&connection, &account, "Archive").id;

    for index in 0..4 {
        let thread = a_thread(&connection, account.id);
        unread_in(&connection, account.id, inbox, thread.id, index * 10);
    }
    for index in 0..3 {
        let thread = a_thread(&connection, account.id);
        unread_in(
            &connection,
            account.id,
            archive,
            thread.id,
            100 + index * 10,
        );
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox);
    assert_eq!(repository.count_of(&query).expect("a count"), 4);
    assert_eq!(repository.page(&query).expect("a page").len(), 4);
    assert_eq!(
        repository.count(account.id).expect("an account count"),
        7,
        "the account still sees every conversation"
    );
}

#[test]
fn a_thread_page_at_an_offset_resumes_where_the_previous_one_stopped() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let threads: Vec<Thread> = (0..6).map(|_| a_thread(&connection, account.id)).collect();
    for (index, thread) in threads.iter().enumerate() {
        unread_in(&connection, account.id, inbox, thread.id, index as i64 * 10);
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox).limit(2);
    let all: Vec<Option<ThreadId>> = repository
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .expect("every row")
        .iter()
        .map(|row| row.id)
        .collect();

    let second: Vec<Option<ThreadId>> = repository
        .page_at(&query, 2)
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

#[test]
fn a_message_belonging_to_no_thread_is_still_a_row() {
    // The list must never hide mail. Threading runs on everything sync files
    // and can still fail — `postio-sync`'s send path discards the result with
    // `let _ =` — and a window built only over `threads` makes every such
    // message *invisible*: no error, no empty state, just mail that is in the
    // store and not on screen.
    //
    // So the folder window is over the representative *message* rather than
    // over `threads`, and a message with no thread is a conversation of one.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let threaded = a_thread(&connection, account.id);
    unread_in(&connection, account.id, inbox, threaded.id, 10);

    // Straight into the table, exactly as a message that was never threaded
    // would sit there.
    let mut orphan = Message::new(account.id, inbox, at(20));
    orphan.subject = Some("Never threaded".to_owned());
    orphan.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    let orphan = MessageRepository::new(&connection)
        .create(&mut orphan)
        .expect("create an unthreaded message");

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
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

#[test]
fn two_unthreaded_messages_are_two_rows_rather_than_one() {
    // The coalesce that lets a null thread id be compared must not make every
    // unthreaded message look like a member of the same conversation.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    for second in [10, 20, 30] {
        let mut message = Message::new(account.id, inbox, at(second));
        message.subject = Some(format!("Never threaded {second}"));
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create an unthreaded message");
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox);
    assert_eq!(repository.page(&query).expect("a page").len(), 3);
    assert_eq!(repository.count_of(&query).expect("a count"), 3);
}
