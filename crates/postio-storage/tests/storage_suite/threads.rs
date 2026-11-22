use chrono::{DateTime, TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{
    AccountId, EmailAddress, Flag, MailboxId, Message, MessageId, Thread, ThreadId,
};
use postio_storage::repository::{
    FlagSource, MessageRepository, ThreadListQuery, ThreadOrder, ThreadRepository,
};
use postio_storage::test_support;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

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

    let stored = threads
        .get(thread.id)
        .await
        .expect("get")
        .expect("the thread");
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

    assert!(
        threads
            .get(ThreadId::new(404))
            .await
            .expect("get")
            .is_none()
    );
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
    threads
        .add_message(thread.id, message.id)
        .await
        .expect("add");

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

    let after_root = threads
        .get(thread.id)
        .await
        .expect("get")
        .expect("the thread");
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

    let after_reply = threads
        .get(thread.id)
        .await
        .expect("get")
        .expect("the thread");
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

    let stored = threads
        .get(thread.id)
        .await
        .expect("get")
        .expect("the thread");
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
            !postio_storage::test_support::sorts(&plan),
            "{order:?}: the drill-in must not sort:\n{plan}"
        );
        assert!(
            plan.contains("idx_messages_thread"),
            "{order:?}: expected the thread index:\n{plan}"
        );
    }
}

#[tokio::test]
async fn the_rows_for_changed_messages_are_their_conversations_and_nothing_else() {
    // #1607: a flag change names message ids, and the list re-read a whole page of correlated
    // subqueries
    // to learn one conversation's state; this reads only the rows those ids touch, in bounded
    // statements.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut members = Vec::new();
    for index in 0..5 {
        let thread = a_thread(&connection, account.id).await;
        for reply in 0..3 {
            let sender = ["ada", "quinn", "tove"][reply as usize];
            let message = message(
                &connection,
                account.id,
                inbox,
                sender,
                index * 1_000 + reply * 10,
            )
            .await;
            ThreadRepository::new(&connection)
                .add_message(thread.id, message.id)
                .await
                .expect("add");
            members.push((thread.id, message.id));
        }
    }
    let lone = message(&connection, account.id, inbox, "ada", 9_000).await;
    let (changed_thread, changed) = members[2 * 3 + 1];
    MessageRepository::new(&connection)
        .set_flags(
            changed,
            &postio_model::FlagSet::from_iter(std::iter::empty::<Flag>()),
            FlagSource::Local,
        )
        .await
        .expect("unread");

    let query = ThreadListQuery::in_mailbox(account.id, inbox);
    postio_storage::test_support::counting::reset();
    let rows = ThreadRepository::new(&connection)
        .rows_for(&query, &[changed, lone.id])
        .await
        .expect("rows");
    let statements = postio_storage::test_support::counting::here().statements;

    assert_eq!(
        rows.len(),
        2,
        "one row per conversation the ids touch: {rows:?}"
    );
    let touched = rows
        .iter()
        .find(|row| row.id == Some(changed_thread))
        .expect("the changed conversation's row");
    assert_eq!(touched.unread_count, 1, "the aggregate the flag moved");
    assert_eq!(touched.message_count, 3);
    assert!(
        touched.latest.is_some() && !touched.participants.is_empty(),
        "the row is the page's shape, representative and participants included"
    );
    let alone = rows
        .iter()
        .find(|row| row.id.is_none())
        .expect("a message with no conversation is its own row");
    assert_eq!(alone.latest.as_ref().map(|latest| latest.id), Some(lone.id));
    assert!(
        statements <= 4,
        "the rows for two ids took {statements} statements; a page of fifty \
         takes four, and this must not scale with the ids"
    );

    let plan = ThreadRepository::new(&connection).explain_rows_for(&query, 1, 1);
    let scans = postio_storage::test_support::counting::scans(&connection, &plan).await;
    assert!(
        scans.is_empty(),
        "the rows-for read scans: {scans:?}\n{plan}"
    );
}

#[tokio::test]
async fn conversations_gone_from_a_folder_are_the_ones_with_no_member_left() {
    // #1607: an archive moved `total_count`, the count's witness, so the next page paid the whole
    // count
    // again; what the count lost is knowable from the removed ids alone.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let thread = a_thread(&connection, account.id).await;
    let mut members = Vec::new();
    for reply in 0..3 {
        let sender = ["ada", "quinn", "tove"][reply as usize];
        let message = message(&connection, account.id, inbox, sender, reply * 10).await;
        ThreadRepository::new(&connection)
            .add_message(thread.id, message.id)
            .await
            .expect("add");
        members.push(message.id);
    }
    let lone = message(&connection, account.id, inbox, "ada", 9_000)
        .await
        .id;
    let messages = MessageRepository::new(&connection);
    let threads = ThreadRepository::new(&connection);

    messages
        .move_to(&[members[0]], archive)
        .await
        .expect("moved");
    assert_eq!(
        threads
            .conversations_gone_from(inbox, &[members[0]])
            .await
            .expect("counted"),
        0,
        "a conversation with members left is still a row"
    );

    messages
        .move_to(&[members[1], members[2], lone], archive)
        .await
        .expect("moved");
    postio_storage::test_support::counting::reset();
    let gone = threads
        .conversations_gone_from(inbox, &[members[1], members[2], lone])
        .await
        .expect("counted");
    let statements = postio_storage::test_support::counting::here().statements;
    assert_eq!(
        gone, 2,
        "the conversation and the lone message each were a row"
    );
    assert!(
        statements <= 2,
        "counting what left took {statements} statements; it must not scale with the folder"
    );
}

#[tokio::test]
async fn a_page_of_threads_costs_a_fixed_number_of_queries() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

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
            )
            .await;
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
        threads
            .add_message(thread.id, message.id)
            .await
            .expect("add");
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
                !postio_storage::test_support::sorts(&plan),
                "{label} / cursor={after}: the thread list must never sort:\n{plan}"
            );
            assert!(
                !plan.contains("SCAN threads") && !plan.contains("SCAN messages"),
                "{label} / cursor={after}: the list must never scan a table:\n{plan}"
            );
            if base.mailbox.is_some() {
                assert!(
                    plan.contains("idx_messages_list"),
                    "{label} / cursor={after}: the folder window must walk \
                     the message list index:\n{plan}"
                );
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

    let merged = threads
        .get(keep.id)
        .await
        .expect("get")
        .expect("the thread");
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

    let stored = threads
        .get(thread.id)
        .await
        .expect("get")
        .expect("still there");
    assert_eq!(stored.message_count, 1);
}

// A folder shows one row per conversation, windowed over `threads`, never over messages collapsed
// afterwards. Whether the thread appears, which message represents it and how much is unread are
// scoped to the folder; the total count is not (ADR 0015 Q2).

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
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

    let thread = a_thread(&connection, account.id).await;
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
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

    let thread = a_thread(&connection, account.id).await;
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

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

/// POSTIO-MEASUREMENT: seeds a large folder to compare statement counts across pages, 28 s.
#[tokio::test]
async fn thread_paging_stays_flat_over_a_large_folder() {
    // The claim ADR 0015 rests on: page k of threads costs what page k of messages costs. Counted,
    // not
    // timed (#100), over 20,000 messages: a row count does not move with the corpus, and more
    // seeding only cost time.
    let database = postio_storage::test_support::temp().await;
    let report = postio_storage::seed::seed_large(&database, 7, 20_000).await;
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

    let mut cursor = first_page.last().expect("a last row").cursor();
    let mut deep = first;
    for _ in 0..10 {
        let (counts, page) = read(&query.clone().after(cursor)).await;
        assert_eq!(page.len(), 50);
        cursor = page.last().expect("a last row").cursor();
        deep = counts;
    }

    // Not exact equality: the correlated subqueries return a row per message in the page's threads,
    // so
    // the count moves with conversation shape. Doubling still rules out the linear case.
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
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;

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
        )
        .await;
    }

    let repository = ThreadRepository::new(&connection);
    let query = ThreadListQuery::in_mailbox(account.id, inbox);
    assert_eq!(repository.count_of(&query).await.expect("a count"), 4);
    assert_eq!(repository.page(&query).await.expect("a page").len(), 4);
    assert_eq!(
        repository
            .count(account.id)
            .await
            .expect("an account count"),
        7,
        "the account still sees every conversation"
    );
}

#[tokio::test]
async fn a_thread_page_at_an_offset_resumes_where_the_previous_one_stopped() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

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
    // The list must never hide mail: threading can fail (the send path discards its result), so the
    // window is over the representative message, and an unthreaded message is a conversation of
    // one.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let threaded = a_thread(&connection, account.id).await;
    unread_in(&connection, account.id, inbox, threaded.id, 10).await;

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

#[tokio::test]
async fn a_folder_is_counted_from_an_index_alone() {
    // #1607, #1610: the folder count was a correlated probe per message (786 ms on a real folder).
    // The
    // plan is the gate: every step reads a covering index and nothing runs a subquery per message.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_, inbox) = test_support::account_with_inbox(&connection).await;
    let sql = ThreadRepository::new(&connection).explain_count_of();
    let plan: Vec<String> = postio_storage::sql::all(
        &connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        [inbox.get()],
        |row| postio_storage::sql::RowExt::col(row, 3),
    )
    .await
    .expect("a plan");
    let uncovered: Vec<&String> = plan
        .iter()
        .filter(|step| !(step.contains("COVERING INDEX") || step.starts_with("USE ")))
        .collect();
    assert!(
        uncovered.is_empty(),
        "a folder count that reads more than an index: {uncovered:?}\n{plan:#?}"
    );
}

#[tokio::test]
async fn a_folder_s_page_boundaries_are_where_its_pages_begin() {
    // #1610: a scrollbar jump was an OFFSET walk, linear in depth; boundaries let it seek, and
    // matter
    // only if resuming after one reads exactly the page OFFSET would.
    let database = test_support::memory().await;
    let seed = postio_storage::seed::seed_large(&database, 7, 2_000).await;
    let connection = database.connect().await.expect("checkout");
    let inbox = seed
        .mailbox(postio_model::MailboxRole::Inbox)
        .expect("an inbox")
        .id;
    let mut query = ThreadListQuery::in_mailbox(seed.account.id, inbox);
    query.limit = 50;
    let threads = ThreadRepository::new(&connection);
    let total = threads.count_of(&query).await.expect("a count");
    let boundaries = threads.boundaries(&query, 50).await.expect("boundaries");
    assert_eq!(
        boundaries.len() as u32,
        (total.saturating_sub(1)) / 50,
        "one boundary per page after the first, in a folder of {total} rows"
    );
    for (offset, cursor) in boundaries.iter().copied().step_by(3) {
        let skipped = threads.page_at(&query, offset).await.expect("by offset");
        let sought = threads
            .page(&ThreadListQuery {
                after: Some(cursor),
                ..query
            })
            .await
            .expect("by cursor");
        let ids = |rows: &[postio_storage::repository::ThreadListRow]| {
            rows.iter().map(|row| row.cursor().id).collect::<Vec<_>>()
        };
        assert_eq!(ids(&sought), ids(&skipped), "the page at {offset}");
    }
}

#[tokio::test]
async fn a_threads_aggregates_are_written_back_and_a_missing_thread_is_refused() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let mut thread = a_thread(&connection, account.id).await;

    thread.subject = Some("tide gate, revised".to_owned());
    thread.message_count = 4;
    thread.unread_count = 1;
    thread.has_attachments = true;
    thread.is_flagged = true;
    thread.first_at = at(10);
    thread.last_at = at(500);
    threads.update(&thread).await.expect("written back");
    let read = threads
        .get(thread.id)
        .await
        .expect("a read")
        .expect("there");
    assert_eq!(read.subject.as_deref(), Some("tide gate, revised"));
    assert_eq!(
        (
            read.message_count,
            read.unread_count,
            read.has_attachments,
            read.is_flagged
        ),
        (4, 1, true, true)
    );
    assert_eq!((read.first_at, read.last_at), (at(10), at(500)));

    let mut missing = thread.clone();
    missing.id = ThreadId::new(thread.id.get() + 1_000);
    assert!(
        threads.update(&missing).await.is_err(),
        "writing back a thread that is not there is an error, not a silent no-op"
    );
    assert!(
        threads.update(&Thread::new(account.id)).await.is_err(),
        "an unpersisted thread has no row to write to"
    );
    assert!(
        threads
            .add_message(thread.id, MessageId::new(9_999))
            .await
            .is_err(),
        "a message that is not there cannot join"
    );
}

#[tokio::test]
async fn a_conversation_knows_its_newest_message_whether_the_person_wrote_in_it_and_what_sleeps() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let sent = test_support::mailbox(&connection, &account, "Sent")
        .await
        .id;
    let threads = ThreadRepository::new(&connection);
    let thread = a_thread(&connection, account.id).await;
    assert_eq!(
        threads.latest_member(thread.id).await.expect("a read"),
        None
    );
    assert!(!threads.took_part(thread.id).await.expect("a read"));

    let first = message(&connection, account.id, inbox, "ada", 10).await;
    let second = message(&connection, account.id, inbox, "grace", 20).await;
    for member in [&first, &second] {
        threads
            .add_message(thread.id, member.id)
            .await
            .expect("joined");
    }
    assert_eq!(
        threads.latest_member(thread.id).await.expect("a read"),
        Some(second.id)
    );
    assert!(
        !threads.took_part(thread.id).await.expect("a read"),
        "nothing of the person's own is in it yet"
    );
    let reply = message(&connection, account.id, sent, "me", 30).await;
    threads
        .add_message(thread.id, reply.id)
        .await
        .expect("joined");
    assert!(threads.took_part(thread.id).await.expect("a read"));

    assert!(
        threads
            .snoozed_messages(thread.id)
            .await
            .expect("a read")
            .is_empty()
    );
    MessageRepository::new(&connection)
        .snooze(&[first.id], at(1_000))
        .await
        .expect("snoozed");
    let asleep = threads.snoozed_messages(thread.id).await.expect("a read");
    assert_eq!(
        asleep.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![first.id]
    );
}

#[tokio::test]
async fn unsnoozing_by_thread_wakes_that_conversations_sleepers_and_no_other() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let messages = MessageRepository::new(&connection);
    let ours = a_thread(&connection, account.id).await;
    let other = a_thread(&connection, account.id).await;
    let mine = message(&connection, account.id, inbox, "ada", 10).await;
    let theirs = message(&connection, account.id, inbox, "grace", 20).await;
    threads.add_message(ours.id, mine.id).await.expect("joined");
    threads
        .add_message(other.id, theirs.id)
        .await
        .expect("joined");
    messages
        .snooze(&[mine.id, theirs.id], at(1_000))
        .await
        .expect("snoozed");

    assert_eq!(messages.unsnooze_threads(&[]).await.expect("a write"), 0);
    assert_eq!(
        messages
            .unsnooze_threads(&[ours.id])
            .await
            .expect("a write"),
        1
    );
    assert!(
        threads
            .snoozed_messages(ours.id)
            .await
            .expect("a read")
            .is_empty()
    );
    assert_eq!(
        threads
            .snoozed_messages(other.id)
            .await
            .expect("a read")
            .len(),
        1,
        "another conversation's sleeper is left alone"
    );
    assert_eq!(
        messages
            .unsnooze_threads(&[ours.id])
            .await
            .expect("a write"),
        0,
        "nothing asleep, nothing to wake"
    );
}

#[tokio::test]
async fn a_conversation_with_no_copies_elsewhere_folds_into_itself() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let thread = a_thread(&connection, account.id).await;
    let member = message(&connection, account.id, inbox, "ada", 10).await;
    threads
        .add_message(thread.id, member.id)
        .await
        .expect("joined");

    assert!(
        threads
            .with_folded_copies(&[])
            .await
            .expect("a read")
            .is_empty()
    );
    assert_eq!(
        threads
            .with_folded_copies(&[thread.id])
            .await
            .expect("a read"),
        vec![thread.id]
    );
}

// `counting` does not see trigger firings, so the budgets below are statements and rows; the
// mailbox-count triggers are covered by the aggregates being right after each write.

mod mute_and_split {
    use std::collections::HashMap;

    use postio_model::{RfcMessageId, ThreadId};
    use postio_storage::repository::ThreadingRepository;
    use postio_storage::test_support::counting::counted_async;

    use super::*;

    struct Scene<'a> {
        connection: &'a Connection,
        account: AccountId,
        inbox: MailboxId,
    }

    impl Scene<'_> {
        async fn file(
            &self,
            rfc: &str,
            references: &[&str],
            minutes: i64,
        ) -> (MessageId, ThreadId) {
            let mut message = Message::new(self.account, self.inbox, at(minutes * 60));
            message.rfc_message_id = Some(RfcMessageId::new(format!("<{rfc}@example.com>")));
            message.references = references
                .iter()
                .map(|id| RfcMessageId::new(format!("<{id}@example.com>")))
                .collect();
            message.in_reply_to = message.references.last().cloned();
            message.subject = Some(if rfc.starts_with('m') {
                "Re: Harbor API".to_owned()
            } else {
                format!("Re: {rfc}")
            });
            message.from = vec![EmailAddress::new(None::<String>, "ben@example.com")];
            message.flags = [Flag::Seen].into_iter().collect();
            let id = MessageRepository::new(self.connection)
                .create(&mut message)
                .await
                .expect("create");
            let threaded = ThreadingRepository::new(self.connection, self.account)
                .thread(&message)
                .await
                .expect("thread");
            (id, threaded.thread_id)
        }

        async fn claims(&self, thread: ThreadId) -> Vec<String> {
            ThreadingRepository::new(self.connection, self.account)
                .claims(thread)
                .await
                .expect("claims")
                .iter()
                .map(|id| {
                    id.as_str()
                        .trim_matches(['<', '>'])
                        .replace("@example.com", "")
                })
                .collect()
        }

        async fn members(&self, thread: ThreadId) -> Vec<MessageId> {
            ThreadRepository::new(self.connection)
                .member_ids(thread)
                .await
                .expect("members")
        }

        async fn count(&self, thread: ThreadId) -> u32 {
            ThreadRepository::new(self.connection)
                .get(thread)
                .await
                .expect("get")
                .expect("there")
                .message_count
        }
    }

    async fn scene(connection: &Connection) -> Scene<'_> {
        let (account, inbox) = test_support::account_with_inbox(connection).await;
        Scene {
            connection,
            account: account.id,
            inbox,
        }
    }

    async fn a_conversation(scene: &Scene<'_>) -> (Vec<MessageId>, ThreadId) {
        let (m1, thread) = scene.file("m1", &[], 1).await;
        let (m2, a) = scene.file("m2", &["m1"], 2).await;
        let (m3, b) = scene.file("m3", &["m1", "m2"], 3).await;
        let (m4, c) = scene.file("m4", &["m1", "m2", "m3"], 4).await;
        let (m5, d) = scene.file("m5", &["m1", "m2", "m3", "m4"], 5).await;
        let (m6, e) = scene.file("m6", &["m1", "m2", "m3"], 6).await;
        assert!([a, b, c, d, e].iter().all(|t| *t == thread), "one thread");
        (vec![m1, m2, m3, m4, m5, m6], thread)
    }

    #[tokio::test]
    async fn an_archived_reply_to_a_muted_thread_does_not_move_its_row() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let scene = scene(&connection).await;
        let mut archive = postio_model::Mailbox::new(scene.account, "Archive", Some('/'));
        postio_storage::repository::MailboxRepository::new(&connection)
            .create(&mut archive)
            .await
            .expect("an archive");
        let archive = archive.id;
        let (m1, muted) = scene.file("m1", &[], 1).await;
        let (_, other) = scene.file("b1", &[], 5).await;
        ThreadRepository::new(&connection)
            .mute(muted)
            .await
            .expect("mute");
        let order = || async {
            ThreadRepository::new(&connection)
                .page(&ThreadListQuery {
                    account_id: scene.account,
                    mailbox: Some(scene.inbox),
                    limit: 10,
                    after: None,
                })
                .await
                .expect("a page")
                .iter()
                .map(|row| row.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(order().await, [Some(other), Some(muted)]);

        let (reply, joined) = scene.file("m2", &["m1"], 10).await;
        assert_eq!(joined, muted);
        postio_storage::actions::relocate(
            &connection,
            scene.account,
            &std::collections::BTreeMap::from([(scene.inbox, vec![reply])]),
            archive,
            postio_storage::actions::Relocation::Move,
            Utc::now(),
        )
        .await
        .expect("archived");

        assert_eq!(
            order().await,
            [Some(other), Some(muted)],
            "the muted thread stays below the newer inbox thread"
        );
        let _ = m1;
    }

    #[tokio::test]
    async fn a_mute_survives_a_merge_and_a_renumbering() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);

        let (_, early) = scene.file("a1", &[], 1).await;
        let (_, late) = scene.file("b2", &["b1"], 3).await;
        assert_ne!(early, late);
        threads.mute(late).await.expect("mute");
        assert_eq!(threads.muted(&[late]).await.expect("muted")[&late], 0);
        assert!(threads.muted(&[early]).await.expect("muted").is_empty());

        let (_, merged) = scene.file("b1", &["a1"], 2).await;
        let (_, merged_again) = scene.file("b3", &["a1", "b1", "b2"], 4).await;
        assert_eq!(merged, early, "the older thread survives");
        assert_eq!(merged_again, early);
        assert_eq!(
            threads.muted(&[early]).await.expect("muted").get(&early),
            Some(&0),
            "still muted after the merge, under the surviving id"
        );

        let members = scene.members(early).await;
        threads.delete(early).await.expect("delete");
        let messages = MessageRepository::new(&connection);
        let mut renumbered = None;
        for id in members {
            let message = messages.get(id).await.expect("read").expect("there");
            let threaded = ThreadingRepository::new(&connection, scene.account)
                .thread(&message)
                .await
                .expect("rethread");
            renumbered = Some(threaded.thread_id);
        }
        let renumbered = renumbered.expect("a thread");
        assert_ne!(renumbered, early);
        assert!(
            threads
                .muted(&[renumbered])
                .await
                .expect("muted")
                .contains_key(&renumbered),
            "still muted under the new id: the key is a claimed Message-ID"
        );

        threads.unmute(renumbered).await.expect("unmute");
        assert!(
            threads
                .muted(&[renumbered])
                .await
                .expect("muted")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn replies_since_counts_up_and_is_read_with_the_mute() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);
        let (_, thread) = scene.file("a1", &[], 1).await;

        threads.mute(thread).await.expect("mute");
        threads.note_muted_replies(thread, 1).await.expect("one");
        threads
            .note_muted_replies(thread, 2)
            .await
            .expect("two more");

        assert_eq!(threads.muted(&[thread]).await.expect("muted")[&thread], 3);
        threads.mute(thread).await.expect("mute again");
        assert_eq!(threads.muted(&[thread]).await.expect("muted")[&thread], 3);
        threads.unmute(thread).await.expect("unmute");
        threads.mute(thread).await.expect("mute afresh");
        assert_eq!(
            threads.muted(&[thread]).await.expect("muted")[&thread],
            0,
            "a fresh mute starts at none"
        );
    }

    #[tokio::test]
    async fn which_threads_of_a_page_are_muted_is_one_statement() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);
        let mut page = Vec::new();
        for n in 0..8 {
            let (_, thread) = scene.file(&format!("t{n}"), &[], n).await;
            page.push(thread);
        }
        threads.mute(page[2]).await.expect("mute");
        threads.mute(page[5]).await.expect("mute");
        let _ = threads.muted(&page).await.expect("warm");

        let mut found = HashMap::new();
        let counts = counted_async(|| async {
            found = threads.muted(&page).await.expect("muted");
        })
        .await;

        assert_eq!(found.len(), 2);
        assert_eq!(counts.statements, 1, "one statement for the whole page");
        assert_eq!(
            counts.rows, 2,
            "it returns the muted rows and no others (trigger firings are not \
             exposed by `counting`)"
        );
        let none = counted_async(|| async {
            threads.muted(&[]).await.expect("an empty page");
        })
        .await;
        assert_eq!(none.statements, 0, "an empty page asks nothing");
    }

    #[tokio::test]
    async fn a_split_moves_the_message_and_its_replies_and_relinks_their_claims() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);
        let (ids, original) = a_conversation(&scene).await;
        assert_eq!(scene.count(original).await, 6);

        let split = threads.split(ids[3]).await.expect("split");

        assert_ne!(split, original);
        assert_eq!(
            scene.members(split).await,
            vec![ids[3], ids[4]],
            "m4 and m5"
        );
        assert_eq!(
            scene.members(original).await,
            vec![ids[0], ids[1], ids[2], ids[5]],
            "the sibling stays"
        );
        assert_eq!(scene.count(split).await, 2, "its aggregate is recomputed");
        assert_eq!(scene.count(original).await, 4, "and so is the original's");
        let mut moved = scene.claims(split).await;
        moved.sort();
        assert_eq!(moved, vec!["m4", "m5"], "the moved messages' own claims");
        assert!(scene.claims(original).await.contains(&"m3".to_owned()));
        assert!(!scene.claims(original).await.contains(&"m4".to_owned()));
    }

    #[tokio::test]
    async fn a_split_costs_its_threads_reads_and_writes_and_no_mailbox_scan() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);
        for n in 0..30 {
            scene.file(&format!("other{n}"), &[], 100 + n).await;
        }
        let (ids, _) = a_conversation(&scene).await;

        let counts = counted_async(|| async {
            threads.split(ids[3]).await.expect("split");
        })
        .await;

        // Reads: the message's row (1), the thread's nodes (6), its subject (1) and the two
        // threads'
        // aggregates (2); writes are not counted.
        assert_eq!(
            (counts.statements, counts.rows),
            SPLIT_BUDGET,
            "a split reads the one thread's members and nothing else: {counts:?}"
        );
    }

    const SPLIT_BUDGET: (usize, usize) = (5, 10);

    #[tokio::test]
    async fn a_later_reply_to_the_split_message_joins_the_split_thread() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);
        let (ids, original) = a_conversation(&scene).await;
        let split = threads.split(ids[3]).await.expect("split");

        let (reply, joined) = scene.file("m7", &["m1", "m2", "m3", "m4"], 7).await;

        assert_eq!(joined, split, "the nearest link's thread");
        assert!(scene.members(split).await.contains(&reply));
        assert_eq!(scene.count(original).await, 4, "nothing merged back");
        assert!(
            !scene.claims(split).await.contains(&"m3".to_owned()),
            "joining the split does not steal the original's claims"
        );
        let (_, stays) = scene.file("m8", &["m1", "m2", "m3"], 8).await;
        assert_eq!(stays, original);
    }

    #[tokio::test]
    async fn one_undo_of_a_split_rejoins_the_messages_and_forgets_the_record() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let threads = ThreadRepository::new(&connection);
        let (ids, original) = a_conversation(&scene).await;
        let split = threads.split(ids[3]).await.expect("split");

        threads.unsplit(split, original).await.expect("undo");

        assert_eq!(scene.count(original).await, 6, "all six are back");
        let mut back = scene.members(original).await;
        back.sort();
        let mut all = ids.clone();
        all.sort();
        assert_eq!(back, all);
        assert!(
            threads.get(split).await.expect("get").is_none(),
            "the split thread is gone"
        );
        let (_, joined) = scene.file("m7", &["m1", "m2", "m3", "m4"], 7).await;
        assert_eq!(
            joined, original,
            "the record is forgotten: back to the original"
        );
        for claim in ["m4", "m5"] {
            assert!(scene.claims(original).await.contains(&claim.to_owned()));
        }
    }

    #[tokio::test]
    async fn the_first_message_of_a_thread_cannot_be_split_off_it() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let scene = scene(&connection).await;
        let (ids, original) = a_conversation(&scene).await;

        let refused = ThreadRepository::new(&connection).split(ids[0]).await;

        assert!(
            refused.is_err(),
            "everything replies to m1: nothing would stay"
        );
        assert_eq!(scene.count(original).await, 6, "and nothing moved");
    }
}

#[tokio::test]
async fn the_muted_lookup_reads_the_mutes_and_seeks_the_links_by_their_key() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let sql = ThreadRepository::explain_muted(5);

    let plan = test_support::plan(&connection, &sql).await;
    let scans = postio_storage::test_support::counting::scans(&connection, &sql).await;

    assert!(
        scans.iter().all(|step| step.contains("thread_mutes")),
        "only the table of mutes is walked, never the links of a long thread:\n{plan}"
    );
    assert!(
        plan.contains("idx_thread_links_lookup") || plan.contains("thread_links"),
        "{plan}"
    );
    assert!(
        plan.contains("account_id=? AND rfc_message_id=?") || plan.contains("account_id=? AND"),
        "the links are sought with both columns of their key bound:\n{plan}"
    );
}

#[tokio::test]
async fn a_thread_across_archive_sent_and_the_inbox_counts_every_message_on_its_inbox_row() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let sent = test_support::mailbox(&connection, &account, "Sent")
        .await
        .id;

    let thread = a_thread(&connection, account.id).await;
    unread_in(&connection, account.id, archive, thread.id, 10).await;
    unread_in(&connection, account.id, archive, thread.id, 11).await;
    unread_in(&connection, account.id, sent, thread.id, 20).await;
    unread_in(&connection, account.id, sent, thread.id, 21).await;
    unread_in(&connection, account.id, inbox, thread.id, 30).await;

    let page = ThreadRepository::new(&connection)
        .page(&ThreadListQuery::in_mailbox(account.id, inbox))
        .await
        .expect("a page of threads");

    assert_eq!(page.len(), 1, "one conversation");
    assert_eq!(
        page[0].message_count, 5,
        "the sent replies and the archived mail are the conversation's too"
    );
    let members = ThreadRepository::new(&connection)
        .members(thread.id)
        .await
        .expect("members");
    assert_eq!(members.len(), 5, "and every one of them is a child");
}

/// Archiving a thread acts on `ThreadRepository::messages`, so it must not name a draft of the
/// thread;
/// the draft stays in Drafts.
#[tokio::test]
async fn the_messages_a_thread_verb_acts_on_leave_a_draft_of_the_thread_out() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let drafts = test_support::mailbox(&connection, &account, "Drafts")
        .await
        .id;
    let thread = a_thread(&connection, account.id).await;
    let received = unread_in(&connection, account.id, inbox, thread.id, 10).await;
    let draft = unread_in(&connection, account.id, drafts, thread.id, 20).await;
    connection
        .execute("UPDATE messages SET draft = 1 WHERE id = ?1", [draft.get()])
        .await
        .expect("mark it a draft");

    let acted_on: Vec<MessageId> = ThreadRepository::new(&connection)
        .messages(thread.id, ThreadOrder::Oldest)
        .await
        .expect("the thread's messages")
        .iter()
        .map(|row| row.id)
        .collect();

    assert_eq!(
        acted_on,
        vec![received],
        "the draft is not archived with it"
    );
}
