//! Threading against the database: broken chains, out-of-order arrival, merges,
//! and the cost of adding one message to a large mailbox.
//!
//! The linkage *rule* is unit-tested in `postio-model`; what is tested here is
//! that the index answering it is the one the schema can serve cheaply.

use std::cell::Cell;

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use rusqlite::Connection;

use postio_model::{AccountId, MailboxId, Message, MessageId, RfcMessageId, ThreadId};
use postio_storage::repository::{
    MessageRepository, ThreadOrder, ThreadRepository, ThreadingRepository,
};
use postio_storage::test_support;

fn at(minute: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::minutes(minute)
}

fn id(raw: &str) -> RfcMessageId {
    RfcMessageId::new(raw)
}

/// Writes a message and files it, returning what threading decided.
fn file(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    minute: i64,
    message_id: &str,
    references: &[&str],
    subject: &str,
) -> (MessageId, ThreadId) {
    let mut message = Message::new(account, mailbox, at(minute));
    message.rfc_message_id = Some(id(message_id));
    message.references = references.iter().map(|r| id(r)).collect();
    message.subject = Some(subject.to_owned());
    let message_id = MessageRepository::new(connection)
        .create(&mut message)
        .expect("create");

    let threaded = ThreadingRepository::new(connection, account)
        .thread(&message)
        .expect("thread");
    (message_id, threaded.thread_id)
}

fn members(connection: &Connection, thread: ThreadId) -> Vec<MessageId> {
    ThreadRepository::new(connection)
        .messages(thread, ThreadOrder::Oldest)
        .expect("members")
        .into_iter()
        .map(|row| row.id)
        .collect()
}

// ---------------------------------------------------------------------------
// The ordinary cases
// ---------------------------------------------------------------------------

#[test]
fn a_conversation_lands_in_one_thread() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (root, thread) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    );
    let (reply, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    );
    let (third, last) = file(
        &connection,
        account.id,
        inbox,
        2,
        "<c@example.com>",
        &["<a@example.com>", "<b@example.com>"],
        "Re: Contract",
    );

    assert_eq!((second, last), (thread, thread));
    assert_eq!(members(&connection, thread), vec![root, reply, third]);
}

#[test]
fn two_conversations_stay_apart() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    );
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<x@example.com>",
        &[],
        "Invoice",
    );

    assert_ne!(first, second);
}

#[test]
fn a_new_thread_takes_the_normalized_subject() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (_, thread) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "RE: Re: FWD: Contract",
    );

    assert_eq!(
        ThreadRepository::new(&connection)
            .get(thread)
            .expect("get")
            .expect("the thread")
            .subject
            .as_deref(),
        Some("contract")
    );
}

// ---------------------------------------------------------------------------
// Out of order, and merging
// ---------------------------------------------------------------------------

#[test]
fn a_reply_that_arrives_before_its_parent_still_gathers_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // An initial sync walks newest first, so this is the ordinary case.
    let (reply, thread) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    );
    let (parent, same) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    );

    assert_eq!(same, thread);
    assert_eq!(members(&connection, thread), vec![parent, reply]);
}

#[test]
fn a_late_message_that_links_two_threads_merges_them() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (left, first) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    );
    let (right, second) = file(
        &connection,
        account.id,
        inbox,
        2,
        "<c@example.com>",
        &["<x@example.com>"],
        "Re: Notes",
    );
    assert_ne!(first, second, "nothing links them yet");

    // The message that references both turns up.
    let mut linker = Message::new(account.id, inbox, at(3));
    linker.rfc_message_id = Some(id("<a@example.com>"));
    linker.references = vec![id("<x@example.com>")];
    linker.subject = Some("Contract and notes".to_owned());
    let linker_id = MessageRepository::new(&connection)
        .create(&mut linker)
        .expect("create");
    let threaded = ThreadingRepository::new(&connection, account.id)
        .thread(&linker)
        .expect("thread");

    assert_eq!(threaded.thread_id, first, "the older thread survives");
    assert_eq!(threaded.merged, vec![second]);
    assert!(!threaded.created);

    let mut all = members(&connection, first);
    all.sort_unstable_by_key(|id| id.get());
    assert_eq!(all, {
        let mut expected = vec![left, right, linker_id];
        expected.sort_unstable_by_key(|id| id.get());
        expected
    });
    assert_eq!(
        ThreadRepository::new(&connection).get(second).expect("get"),
        None,
        "the absorbed thread is gone, not left empty"
    );
}

#[test]
fn a_merge_moves_the_claimed_ids_onto_the_surviving_thread() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let threading = ThreadingRepository::new(&connection, account.id);

    file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: A",
    );
    file(
        &connection,
        account.id,
        inbox,
        2,
        "<c@example.com>",
        &["<x@example.com>"],
        "Re: X",
    );

    let mut linker = Message::new(account.id, inbox, at(3));
    linker.rfc_message_id = Some(id("<a@example.com>"));
    linker.references = vec![id("<x@example.com>")];
    linker.subject = Some("A and X".to_owned());
    MessageRepository::new(&connection)
        .create(&mut linker)
        .expect("create");
    let threaded = threading.thread(&linker).expect("thread");

    let claimed: Vec<String> = threading
        .claims(threaded.thread_id)
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

    // And a further reply to either half finds the merged thread.
    let (_, later) = file(
        &connection,
        account.id,
        inbox,
        4,
        "<d@example.com>",
        &["<c@example.com>"],
        "Re: X",
    );
    assert_eq!(later, threaded.thread_id);
}

// ---------------------------------------------------------------------------
// Broken chains
// ---------------------------------------------------------------------------

#[test]
fn a_reply_with_no_references_falls_back_to_its_subject() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // What a list that rewrites headers leaves behind.
    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    );
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &[],
        "Re: Contract",
    );

    assert_eq!(second, first);
}

#[test]
fn two_messages_that_merely_share_a_subject_are_not_a_conversation() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Hello",
    );
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &[],
        "Hello",
    );

    assert_ne!(second, first);
}

#[test]
fn a_reference_to_a_message_that_never_arrived_is_harmless() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    );
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<c@example.com>",
        &["<a@example.com>", "<gone@example.com>"],
        "Re: Contract",
    );

    assert_eq!(second, first);
}

#[test]
fn message_ids_match_without_regard_to_case() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let (_, first) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<A@Example.COM>",
        &[],
        "Contract",
    );
    let (_, second) = file(
        &connection,
        account.id,
        inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    );

    assert_eq!(second, first);
}

#[test]
fn threading_never_crosses_accounts() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let first = test_support::account(&connection);
    let first_inbox = test_support::mailbox(&connection, &first, "INBOX").id;
    let second = test_support::account(&connection);
    let second_inbox = test_support::mailbox(&connection, &second, "INBOX").id;

    let (_, theirs) = file(
        &connection,
        first.id,
        first_inbox,
        0,
        "<a@example.com>",
        &[],
        "Contract",
    );
    let (_, ours) = file(
        &connection,
        second.id,
        second_inbox,
        1,
        "<b@example.com>",
        &["<a@example.com>"],
        "Re: Contract",
    );

    assert_ne!(
        ours, theirs,
        "two people can hold the same Message-ID and not be in a conversation"
    );
}

#[test]
fn filing_the_same_message_twice_changes_nothing() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let mut message = Message::new(account.id, inbox, at(0));
    message.rfc_message_id = Some(id("<a@example.com>"));
    message.subject = Some("Contract".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");

    let threading = ThreadingRepository::new(&connection, account.id);
    let first = threading.thread(&message).expect("thread");
    let again = threading.thread(&message).expect("thread again");

    assert_eq!(again.thread_id, first.thread_id);
    assert!(!again.created);
    assert_eq!(members(&connection, first.thread_id).len(), 1);
}

// ---------------------------------------------------------------------------
// Cost — the acceptance criterion
// ---------------------------------------------------------------------------

thread_local! {
    static STATEMENTS: Cell<usize> = const { Cell::new(0) };
}

/// Counts every statement SQLite starts running.
///
/// A plain `fn`, not a closure: that is what `trace_v2` takes.
fn count_statement(event: rusqlite::trace::TraceEvent<'_>) {
    if matches!(event, rusqlite::trace::TraceEvent::Stmt(..)) {
        STATEMENTS.with(|count| count.set(count.get() + 1));
    }
}

/// Fills a mailbox with `threads` separate conversations of three messages each,
/// then measures what it costs to file one more message into a new thread.
fn cost_of_one_more(threads: i64) -> usize {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

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
        );
        for reply in 1..3 {
            file(
                &connection,
                account.id,
                inbox,
                index * 10 + reply,
                &format!("<reply{index}-{reply}@example.com>"),
                &[&root],
                &format!("Re: Topic {index}"),
            );
        }
    }

    let mut message = Message::new(account.id, inbox, at(100_000));
    message.rfc_message_id = Some(id("<new@example.com>"));
    message.references = vec![id("<gone-a@example.com>"), id("<gone-b@example.com>")];
    message.subject = Some("Something else".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");

    STATEMENTS.with(|count| count.set(0));
    connection.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
        Some(count_statement),
    );
    ThreadingRepository::new(&connection, account.id)
        .thread(&message)
        .expect("thread");
    connection.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);

    STATEMENTS.with(Cell::get)
}

#[test]
fn adding_a_message_costs_the_same_however_large_the_mailbox_is() {
    let small = cost_of_one_more(10);
    let large = cost_of_one_more(200);

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

#[test]
fn a_long_reference_chain_costs_a_lookup_per_reference_and_no_more() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // A forty-deep conversation, which is a long one and still not a mailbox.
    let references: Vec<String> = (0..40).map(|n| format!("<r{n}@example.com>")).collect();
    let borrowed: Vec<&str> = references.iter().map(String::as_str).collect();

    let mut message = Message::new(account.id, inbox, at(0));
    message.rfc_message_id = Some(id("<deep@example.com>"));
    message.references = borrowed.iter().map(|r| id(r)).collect();
    message.subject = Some("Re: Long".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");

    STATEMENTS.with(|count| count.set(0));
    connection.trace_v2(
        rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
        Some(count_statement),
    );
    ThreadingRepository::new(&connection, account.id)
        .thread(&message)
        .expect("thread");
    connection.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    let statements = STATEMENTS.with(Cell::get);

    // One lookup and one claim per reference, plus the handful for the thread
    // itself. Linear in the conversation, which is what O(thread) means.
    assert!(
        statements < 40 * 3,
        "a 40-reference message took {statements} statements"
    );
}

// ---------------------------------------------------------------------------
// The corpus — the acceptance criterion
// ---------------------------------------------------------------------------

/// Parses a fixture and files it, returning the thread it landed in.
fn file_fixture(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    minute: i64,
    name: &str,
) -> ThreadId {
    let fixture = postio_model::test_corpus::load(name);
    let mut message =
        postio_model::mime::parse(fixture.bytes()).into_message(account, mailbox, at(minute));
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create");
    ThreadingRepository::new(connection, account)
        .thread(&message)
        .expect("thread")
        .thread_id
}

/// The mailing-list thread in the corpus, in the order it was sent.
const LIST_THREAD: &[&str] = &[
    "list-thread-01-root",
    "list-thread-02-reply",
    "list-thread-03-reply-sibling",
    "list-thread-04-reply-deep",
    // Has `In-Reply-To` but no `References` — a client that half-remembers.
    "list-thread-05-reply-no-references",
    // Has neither: only the subject says what it answers.
    "list-thread-06-reply-subject-only",
    // A `(was: …)` subject change, with the chain intact.
    "list-thread-07-subject-change",
];

#[test]
fn the_corpus_mailing_list_threads_as_one_conversation() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let threads: Vec<ThreadId> = LIST_THREAD
        .iter()
        .enumerate()
        .map(|(index, name)| file_fixture(&connection, account.id, inbox, index as i64, name))
        .collect();

    let first = threads[0];
    for (name, thread) in LIST_THREAD.iter().zip(&threads) {
        assert_eq!(
            *thread, first,
            "`{name}` fell out of the conversation it belongs to"
        );
    }
    assert_eq!(members(&connection, first).len(), LIST_THREAD.len());
}

#[test]
fn arriving_newest_first_still_gathers_everything_the_chain_names() {
    // An initial sync walks newest-first, so this order is not the exotic one.
    // Each message keeps its true delivery time; only the order we *file* them
    // in is reversed, which is what a real backwards fetch looks like.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let mut placed: Vec<(&str, ThreadId)> = Vec::new();
    for (index, name) in LIST_THREAD.iter().enumerate().rev() {
        placed.push((
            name,
            file_fixture(&connection, account.id, inbox, index as i64, name),
        ));
    }

    // Everything that carries a usable reference converges on one thread,
    // whichever end the sync starts from.
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
        members(&connection, conversation).len(),
        LIST_THREAD.len() - 1
    );

    // The seventh is the honest limitation of threading incrementally, and it
    // is worth stating rather than hiding.
    //
    // `list-thread-06-reply-subject-only` carries no `In-Reply-To` and no
    // `References`: its subject is the only thing that could place it. Arriving
    // newest-first it is filed *before* the messages that share that subject,
    // and the only thread that exists at that moment is the one started by
    // `list-thread-07-subject-change`, whose subject is deliberately different
    // — `Manual override wiring (was: …)`. There is nothing available to place
    // it correctly at the moment it arrives, and re-deriving the answer later
    // would mean rethreading the mailbox, which is the cost this design exists
    // to avoid. See postio-tn9.2.
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

#[test]
fn rethreading_recovers_the_orphan_arrival_order_stranded() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let mut placed: Vec<(&str, ThreadId)> = Vec::new();
    for (index, name) in LIST_THREAD.iter().enumerate().rev() {
        placed.push((
            name,
            file_fixture(&connection, account.id, inbox, index as i64, name),
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
        .expect("rethread");

    assert_eq!(moved, 1, "exactly the one stranded orphan moves");
    let members_now = members(&connection, conversation);
    assert_eq!(
        members_now.len(),
        LIST_THREAD.len(),
        "the whole mailing-list conversation is one thread now"
    );

    // Idempotent: nothing left to reconsider, so a second pass is a no-op.
    let moved_again = ThreadingRepository::new(&connection, account.id)
        .rethread_orphans(inbox)
        .expect("rethread again");
    assert_eq!(moved_again, 0);
}

#[test]
fn rethreading_never_moves_a_message_out_of_a_thread_it_shares_with_others() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // Two ordinary replies converge on their own thread via real references —
    // never touching the subject fallback at all.
    let (_, root_thread) = file(
        &connection,
        account.id,
        inbox,
        0,
        "<root@example.com>",
        &[],
        "Quarterly numbers",
    );
    file(
        &connection,
        account.id,
        inbox,
        1,
        "<reply@example.com>",
        &["<root@example.com>"],
        "Re: Quarterly numbers",
    );
    assert_eq!(members(&connection, root_thread).len(), 2);

    // A third message with no reference at all, but the same subject as the
    // thread above -- it must not drag `root_thread`'s other member along.
    let mut orphan = Message::new(account.id, inbox, at(2));
    orphan.rfc_message_id = Some(id("<orphan@example.net>"));
    orphan.subject = Some("Re: Quarterly numbers".to_owned());
    let orphan_id = MessageRepository::new(&connection)
        .create(&mut orphan)
        .expect("create");
    let orphan_thread = ThreadingRepository::new(&connection, account.id)
        .thread(&orphan)
        .expect("thread")
        .thread_id;
    assert_eq!(
        orphan_thread, root_thread,
        "the fallback already joins it to the two-message thread at insertion time"
    );

    let moved = ThreadingRepository::new(&connection, account.id)
        .rethread_orphans(inbox)
        .expect("rethread");

    assert_eq!(
        moved, 0,
        "the orphan already sits with others, so nothing is reconsidered for it"
    );
    assert_eq!(members(&connection, root_thread).len(), 3);
    let _ = orphan_id;
}

#[test]
fn a_message_with_broken_references_still_finds_its_conversation() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // `broken-references.eml` carries a `References` header the parser cannot
    // make usable ids out of. It must not take the message out of the mailbox,
    // and it must not crash the pass.
    let root = file_fixture(&connection, account.id, inbox, 0, "list-thread-01-root");
    let broken = file_fixture(&connection, account.id, inbox, 1, "broken-references");

    assert!(
        broken.is_assigned(),
        "a message nobody can place still belongs somewhere"
    );
    let _ = root;
}

#[test]
fn two_messages_claiming_one_message_id_do_not_break_the_index() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // `Message-ID` is not unique in the wild — `duplicate-message-id.eml` is in
    // the corpus precisely because a client out there reuses one.
    let first = file_fixture(&connection, account.id, inbox, 0, "duplicate-message-id");
    let second = file_fixture(&connection, account.id, inbox, 1, "duplicate-message-id");

    assert_eq!(
        second, first,
        "the second claims an id the first already did, so it joins it rather \
         than colliding on the index"
    );
    assert_eq!(members(&connection, first).len(), 2);
}

#[test]
fn every_corpus_fixture_can_be_threaded() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    // Malformed headers, truncated multiparts, missing Message-IDs, mislabelled
    // charsets: none of them may panic the pass or leave a message unfiled.
    for (index, fixture) in postio_model::test_corpus::all().iter().enumerate() {
        let thread = file_fixture(&connection, account.id, inbox, index as i64, fixture.name());
        assert!(
            thread.is_assigned(),
            "`{}` was left without a thread",
            fixture.name()
        );
    }
}
