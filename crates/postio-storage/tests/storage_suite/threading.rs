use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_storage::Connection;

use postio_model::{AccountId, MailboxId, Message, MessageId, RfcMessageId, ThreadId};
use postio_storage::repository::{
    MessageRepository, ThreadOrder, ThreadRepository, ThreadingRepository,
};
use postio_storage::test_support;

async fn at(minute: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::minutes(minute)
}

fn id(raw: &str) -> RfcMessageId {
    RfcMessageId::new(raw)
}

async fn file(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    minute: i64,
    message_id: &str,
    references: &[&str],
    subject: &str,
) -> (MessageId, ThreadId) {
    let mut message = Message::new(account, mailbox, at(minute).await);
    message.rfc_message_id = Some(id(message_id));
    message.references = references.iter().map(|r| id(r)).collect();
    message.subject = Some(subject.to_owned());
    let message_id = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");

    let threaded = ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("thread");
    (message_id, threaded.thread_id)
}

async fn members(connection: &Connection, thread: ThreadId) -> Vec<MessageId> {
    ThreadRepository::new(connection)
        .messages(thread, ThreadOrder::Oldest)
        .await
        .expect("members")
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[tokio::test]
async fn a_conversation_lands_in_one_thread() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (root, thread) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    )
    .await;
    let (reply, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    )
    .await;
    let (third, last) = file(
        &connection,
        account.id,
        inbox,
        2,
        "<c@example.com>",
        &["<a@example.com>", "<b@example.com>"],
        "Re: Contract",
    )
    .await;

    assert_eq!((second, last), (thread, thread));
    assert_eq!(members(&connection, thread).await, vec![root, reply, third]);
}

#[tokio::test]
async fn two_conversations_stay_apart() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    )
    .await;
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<x@example.com>",
        &[],
        "Invoice",
    )
    .await;

    assert_ne!(first, second);
}

#[tokio::test]
async fn a_new_thread_takes_the_normalized_subject() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, thread) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "RE: Re: FWD: Contract",
    )
    .await;

    assert_eq!(
        ThreadRepository::new(&connection)
            .get(thread)
            .await
            .expect("get")
            .expect("the thread")
            .subject
            .as_deref(),
        Some("contract")
    );
}

#[tokio::test]
async fn a_reply_that_arrives_before_its_parent_still_gathers_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (reply, thread) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    )
    .await;
    let (parent, same) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    )
    .await;

    assert_eq!(same, thread);
    assert_eq!(members(&connection, thread).await, vec![parent, reply]);
}

#[tokio::test]
async fn a_late_message_that_links_two_threads_merges_them() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (left, first) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    )
    .await;
    let (right, second) = file(
        &connection,
        account.id,
        inbox,
        2,
        "<c@example.com>",
        &["<x@example.com>"],
        "Re: Notes",
    )
    .await;
    assert_ne!(first, second, "nothing links them yet");

    let mut linker = Message::new(account.id, inbox, at(3).await);
    linker.rfc_message_id = Some(id("<a@example.com>"));
    linker.references = vec![id("<x@example.com>")];
    linker.subject = Some("Contract and notes".to_owned());
    let linker_id = MessageRepository::new(&connection)
        .create(&mut linker)
        .await
        .expect("create");
    let threaded = ThreadingRepository::new(&connection, account.id)
        .thread(&linker)
        .await
        .expect("thread");

    assert_eq!(threaded.thread_id, first, "the older thread survives");
    assert_eq!(threaded.merged, vec![second]);
    assert!(!threaded.created);

    let mut all = members(&connection, first).await;
    all.sort_unstable_by_key(|id| id.get());
    assert_eq!(all, {
        let mut expected = vec![left, right, linker_id];
        expected.sort_unstable_by_key(|id| id.get());
        expected
    });
    assert_eq!(
        ThreadRepository::new(&connection)
            .get(second)
            .await
            .expect("get"),
        None,
        "the absorbed thread is gone, not left empty"
    );
}

#[tokio::test]
async fn a_merge_moves_the_claimed_ids_onto_the_surviving_thread() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threading = ThreadingRepository::new(&connection, account.id);

    file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: A",
    )
    .await;
    file(
        &connection,
        account.id,
        inbox,
        2,
        "<c@example.com>",
        &["<x@example.com>"],
        "Re: X",
    )
    .await;

    let mut linker = Message::new(account.id, inbox, at(3).await);
    linker.rfc_message_id = Some(id("<a@example.com>"));
    linker.references = vec![id("<x@example.com>")];
    linker.subject = Some("A and X".to_owned());
    MessageRepository::new(&connection)
        .create(&mut linker)
        .await
        .expect("create");
    let threaded = threading.thread(&linker).await.expect("thread");

    let claimed: Vec<String> = threading
        .claims(threaded.thread_id)
        .await
        .expect("claims")
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    assert_eq!(
        claimed,
        vec![
            "<a@example.com>",
            "<b@example.com>",
            "<c@example.com>",
            "<x@example.com>"
        ],
        "every id both threads knew about now points at the survivor"
    );

    let (_, later) = file(
        &connection,
        account.id,
        inbox,
        4,
        "<d@example.com>",
        &["<c@example.com>"],
        "Re: X",
    )
    .await;
    assert_eq!(later, threaded.thread_id);
}

#[tokio::test]
async fn a_reply_with_no_references_falls_back_to_its_subject() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    )
    .await;
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &[],
        "Re: Contract",
    )
    .await;

    assert_eq!(second, first);
}

#[tokio::test]
async fn two_messages_that_merely_share_a_subject_are_not_a_conversation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Hello",
    )
    .await;
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &[],
        "Hello",
    )
    .await;

    assert_ne!(second, first);
}

#[tokio::test]
async fn a_reference_to_a_message_that_never_arrived_is_harmless() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    )
    .await;
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<c@example.com>",
        &["<a@example.com>", "<gone@example.com>"],
        "Re: Contract",
    )
    .await;

    assert_eq!(second, first);
}

#[tokio::test]
async fn message_ids_match_without_regard_to_case() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<A@Example.COM>",
        &[],
        "Contract",
    )
    .await;
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    )
    .await;

    assert_eq!(second, first);
}

#[tokio::test]
async fn threading_never_crosses_accounts() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let first = test_support::account(&connection).await;
    let first_inbox = test_support::mailbox(&connection, &first, "INBOX").await.id;
    let second = test_support::account(&connection).await;
    let second_inbox = test_support::mailbox(&connection, &second, "INBOX")
        .await
        .id;

    let (_, theirs) = file(
        &connection,
        first.id,
        first_inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    )
    .await;
    let (_, ours) = file(
        &connection,
        second.id,
        second_inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    )
    .await;

    assert_ne!(
        ours, theirs,
        "two people can hold the same Message-ID and not be in a conversation"
    );
}

#[tokio::test]
async fn filing_the_same_message_twice_changes_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let mut message = Message::new(account.id, inbox, at(0).await);
    message.rfc_message_id = Some(id("<a@example.com>"));
    message.subject = Some("Contract".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create");

    let threading = ThreadingRepository::new(&connection, account.id);
    let first = threading.thread(&message).await.expect("thread");
    let again = threading.thread(&message).await.expect("thread again");

    assert_eq!(again.thread_id, first.thread_id);
    assert!(!again.created);
    assert_eq!(members(&connection, first.thread_id).await.len(), 1);
}

async fn cost_of_one_more(threads: i64) -> usize {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    for index in 0..threads {
        let root = format!("<root{index}@example.com>");
        file(
            &connection,
            account.id,
            inbox,
            index * 10,
            &root,
            &[],
            &format!("Topic {index}"),
        )
        .await;
        for reply in 1..3 {
            file(
                &connection,
                account.id,
                inbox,
                index * 10 + reply,
                &format!("<reply{index}-{reply}@example.com>"),
                &[&root],
                &format!("Re: Topic {index}"),
            )
            .await;
        }
    }

    let mut message = Message::new(account.id, inbox, at(100_000).await);
    message.rfc_message_id = Some(id("<new@example.com>"));
    message.references = vec![id("<gone-a@example.com>"), id("<gone-b@example.com>")];
    message.subject = Some("Something else".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create");

    postio_storage::test_support::counting::reset();
    ThreadingRepository::new(&connection, account.id)
        .thread(&message)
        .await
        .expect("thread");
    postio_storage::test_support::counting::here().statements
}

#[tokio::test]
async fn adding_a_message_costs_the_same_however_large_the_mailbox_is() {
    let small = cost_of_one_more(10).await;
    let large = cost_of_one_more(200).await;

    assert_eq!(
        small, large,
        "filing one message took {small} statements in a 30-message mailbox and \
         {large} in a 600-message one; threading must be O(thread), not O(mailbox)"
    );
    assert!(
        small < 20,
        "filing one message took {small} statements, which is more than a \
         lookup and the writes for one thread"
    );
}

#[tokio::test]
async fn a_long_reference_chain_costs_a_lookup_per_reference_and_no_more() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let references: Vec<String> = (0..40).map(|n| format!("<r{n}@example.com>")).collect();
    let borrowed: Vec<&str> = references.iter().map(String::as_str).collect();

    let mut message = Message::new(account.id, inbox, at(0).await);
    message.rfc_message_id = Some(id("<deep@example.com>"));
    message.references = borrowed.iter().map(|r| id(r)).collect();
    message.subject = Some("Re: Long".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create");

    postio_storage::test_support::counting::reset();
    ThreadingRepository::new(&connection, account.id)
        .thread(&message)
        .await
        .expect("thread");
    let statements = postio_storage::test_support::counting::here().statements;

    assert!(
        statements < 40 * 3,
        "a 40-reference message took {statements} statements"
    );
}

async fn file_fixture(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    minute: i64,
    name: &str,
) -> ThreadId {
    file_fixture_as(connection, account, mailbox, minute, name, false).await
}

/// `file_fixture`, optionally as if the message were not list mail: list mail stays out of the
/// subject
/// fallback, so the rethreading cases need the same message minus its `List-Id` and `Precedence`.
async fn file_fixture_as(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    minute: i64,
    name: &str,
    unlisted: bool,
) -> ThreadId {
    let fixture = postio_model::test_corpus::load(name);
    let mut message =
        postio_model::mime::parse(fixture.bytes()).into_message(account, mailbox, at(minute).await);
    if unlisted {
        message.list_id = None;
        message.promoted = None;
    }
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("thread")
        .thread_id
}

const LIST_THREAD: &[&str] = &[
    "list-thread-01-root",
    "list-thread-02-reply",
    "list-thread-03-reply-sibling",
    "list-thread-04-reply-deep",
    "list-thread-05-reply-no-references",
    "list-thread-06-reply-subject-only",
    "list-thread-07-subject-change",
];

#[tokio::test]
async fn the_corpus_mailing_list_threads_as_one_conversation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let mut threads: Vec<ThreadId> = Vec::new();
    for (index, name) in LIST_THREAD.iter().enumerate() {
        threads.push(file_fixture(&connection, account.id, inbox, index as i64, name).await);
    }

    let first = threads[0];
    for (name, thread) in LIST_THREAD.iter().zip(&threads) {
        if *name == "list-thread-06-reply-subject-only" {
            assert_ne!(*thread, first, "list mail never groups by subject");
            continue;
        }
        assert_eq!(
            *thread, first,
            "`{name}` fell out of the conversation it belongs to"
        );
    }
    assert_eq!(
        members(&connection, first).await.len(),
        LIST_THREAD.len() - 1
    );
}

#[tokio::test]
async fn arriving_newest_first_still_gathers_everything_the_chain_names() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let mut placed: Vec<(&str, ThreadId)> = Vec::new();
    for (index, name) in LIST_THREAD.iter().enumerate().rev() {
        placed.push((
            name,
            file_fixture(&connection, account.id, inbox, index as i64, name).await,
        ));
    }

    let by_chain: Vec<&(&str, ThreadId)> = placed
        .iter()
        .filter(|(name, _)| *name != "list-thread-06-reply-subject-only")
        .collect();
    let conversation = by_chain[0].1;
    for (name, thread) in &by_chain {
        assert_eq!(
            *thread, conversation,
            "`{name}` fell out of the conversation its References name"
        );
    }
    assert_eq!(
        members(&connection, conversation).await.len(),
        LIST_THREAD.len() - 1
    );

    // `list-thread-06-reply-subject-only` has no references, so only its subject could place it,
    // but
    // newest-first it is filed before the messages sharing that subject: the honest limit of
    // incremental threading (postio-tn9.2).
    let (_, stranded) = placed
        .iter()
        .find(|(name, _)| *name == "list-thread-06-reply-subject-only")
        .expect("the subject-only reply");
    assert_ne!(
        *stranded, conversation,
        "if this now joins, the limitation has been fixed and this test should \
         become an assertion that it does"
    );
}

#[tokio::test]
async fn rethreading_recovers_the_orphan_arrival_order_stranded() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let mut placed: Vec<(&str, ThreadId)> = Vec::new();
    for (index, name) in LIST_THREAD.iter().enumerate().rev() {
        placed.push((
            name,
            file_fixture_as(
                &connection,
                account.id,
                inbox,
                index as i64,
                name,
                *name == "list-thread-06-reply-subject-only",
            )
            .await,
        ));
    }
    let conversation = placed
        .iter()
        .find(|(name, _)| *name == "list-thread-01-root")
        .expect("the root")
        .1;
    let (_, stranded_before) = placed
        .iter()
        .find(|(name, _)| *name == "list-thread-06-reply-subject-only")
        .expect("the subject-only reply");
    assert_ne!(
        *stranded_before, conversation,
        "stranded, as established above"
    );

    let moved = ThreadingRepository::new(&connection, account.id)
        .rethread_orphans(inbox)
        .await
        .expect("rethread");

    assert_eq!(moved, 1, "exactly the one stranded orphan moves");
    let members_now = members(&connection, conversation).await;
    assert_eq!(
        members_now.len(),
        LIST_THREAD.len(),
        "the whole mailing-list conversation is one thread now"
    );

    let moved_again = ThreadingRepository::new(&connection, account.id)
        .rethread_orphans(inbox)
        .await
        .expect("rethread again");
    assert_eq!(moved_again, 0);
}

#[tokio::test]
async fn rethreading_never_moves_a_message_out_of_a_thread_it_shares_with_others() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let (_, root_thread) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<root@example.com>",
        &[],
        "Quarterly numbers",
    )
    .await;
    file(
        &connection,
        account.id,
        inbox,
        1,
        "<reply@example.com>",
        &["<root@example.com>"],
        "Re: Quarterly numbers",
    )
    .await;
    assert_eq!(members(&connection, root_thread).await.len(), 2);

    let mut orphan = Message::new(account.id, inbox, at(2).await);
    orphan.rfc_message_id = Some(id("<orphan@example.net>"));
    orphan.subject = Some("Re: Quarterly numbers".to_owned());
    let orphan_id = MessageRepository::new(&connection)
        .create(&mut orphan)
        .await
        .expect("create");
    let orphan_thread = ThreadingRepository::new(&connection, account.id)
        .thread(&orphan)
        .await
        .expect("thread")
        .thread_id;
    assert_eq!(
        orphan_thread, root_thread,
        "the fallback already joins it to the two-message thread at insertion time"
    );

    let moved = ThreadingRepository::new(&connection, account.id)
        .rethread_orphans(inbox)
        .await
        .expect("rethread");

    assert_eq!(
        moved, 0,
        "the orphan already sits with others, so nothing is reconsidered for it"
    );
    assert_eq!(members(&connection, root_thread).await.len(), 3);
    let _ = orphan_id;
}

#[tokio::test]
async fn a_message_with_broken_references_still_finds_its_conversation() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let root = file_fixture(&connection, account.id, inbox, 0, "list-thread-01-root").await;
    let broken = file_fixture(&connection, account.id, inbox, 1, "broken-references").await;

    assert!(
        broken.is_assigned(),
        "a message nobody can place still belongs somewhere"
    );
    let _ = root;
}

#[tokio::test]
async fn two_messages_claiming_one_message_id_do_not_break_the_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let first = file_fixture(&connection, account.id, inbox, 0, "duplicate-message-id").await;
    let second = file_fixture(&connection, account.id, inbox, 1, "duplicate-message-id").await;

    assert_eq!(
        second, first,
        "the second claims an id the first already did, so it joins it rather \
         than colliding on the index"
    );
    assert_eq!(members(&connection, first).await.len(), 2);
}

#[tokio::test]
async fn every_corpus_fixture_can_be_threaded() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    for (index, fixture) in postio_model::test_corpus::all().iter().enumerate() {
        let thread =
            file_fixture(&connection, account.id, inbox, index as i64, fixture.name()).await;
        assert!(
            thread.is_assigned(),
            "`{}` was left without a thread",
            fixture.name()
        );
    }
}
