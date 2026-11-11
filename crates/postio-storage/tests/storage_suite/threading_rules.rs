//! Spec 013's grouping rules, over the `ThreadingRules` corpus: the subject
//! fallback joins one thread inside a 7-day window and never merges, list mail
//! never groups by subject, and references still thread across folders and
//! across the account's own replies.
//!
//! The rule itself is unit-tested in `postio-model`; this is the other half,
//! that the index answering it gives the same answers from the database, and
//! at the same cost however many threads share a subject.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::test_corpus::{self, Category};
use postio_model::{AccountId, MailboxId, Message, ThreadId};
use postio_storage::Connection;
use postio_storage::repository::{MessageRepository, ThreadingRepository};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, install};

/// Files the named fixtures in order, each in the folder it is paired with,
/// at the time its `Date:` header names. Returns each one's thread.
async fn file_all(
    connection: &Connection,
    account: AccountId,
    folders: &dyn Fn(&str) -> MailboxId,
    names: &[&str],
) -> Vec<ThreadId> {
    let mut threads = Vec::new();
    for name in names {
        let mut message = test_corpus::load(name).parse();
        message.account_id = account;
        message.mailbox_id = folders(name);
        message.received_at = message.date.expect("every threading fixture is dated");
        MessageRepository::new(connection)
            .create(&mut message)
            .await
            .expect("create");
        let threaded = ThreadingRepository::new(connection, account)
            .thread(&message)
            .await
            .expect("thread");
        threads.push(threaded.thread_id);
    }
    threads
}

async fn in_inbox(names: &[&str]) -> Vec<ThreadId> {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    file_all(&connection, account.id, &|_| inbox, names).await
}

#[test]
fn the_rules_corpus_is_all_there() {
    assert_eq!(
        test_corpus::by_category(Category::ThreadingRules).len(),
        17,
        "a fixture was added or lost; the cases below name them one by one"
    );
}

#[tokio::test]
async fn two_hello_replies_a_month_apart_are_two_threads() {
    let threads = in_inbox(&["thread-hello-30d-1", "thread-hello-30d-2"]).await;
    assert_ne!(threads[0], threads[1]);
}

#[tokio::test]
async fn two_hello_replies_a_month_apart_are_two_threads_arriving_newest_first() {
    // An initial sync files newest first: the November message is stored
    // before the October one, which must not join it.
    let threads = in_inbox(&["thread-hello-30d-2", "thread-hello-30d-1"]).await;
    assert_ne!(threads[0], threads[1]);
}

#[tokio::test]
async fn two_hello_replies_three_days_apart_are_one_thread() {
    let threads = in_inbox(&["thread-hello-3d-1", "thread-hello-3d-2"]).await;
    assert_eq!(threads[0], threads[1]);
}

#[tokio::test]
async fn two_newsletter_issues_are_two_threads() {
    let threads = in_inbox(&["thread-newsletter-1-issue", "thread-newsletter-2-issue"]).await;
    assert_ne!(threads[0], threads[1], "list mail never groups by subject");
}

#[tokio::test]
async fn a_list_discussion_is_one_thread_by_references() {
    let threads = in_inbox(&[
        "thread-list-1-root",
        "thread-list-2-reply",
        "thread-list-3-reply",
    ])
    .await;
    assert_eq!(threads[0], threads[1]);
    assert_eq!(threads[1], threads[2]);
}

#[tokio::test]
async fn a_fork_is_one_thread() {
    let threads = in_inbox(&[
        "thread-fork-1-root",
        "thread-fork-2-reply-a",
        "thread-fork-3-reply-b",
        "thread-fork-4-reply-to-a",
    ])
    .await;
    assert!(threads.iter().all(|thread| *thread == threads[0]));
}

#[tokio::test]
async fn a_reply_filed_in_another_folder_joins_its_parent() {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let sent = test_support::mailbox(&connection, &account, "Sent").await;

    let folders = |name: &str| match name {
        "thread-archive-2-reply" => archive.id,
        "thread-sent-2-reply" => sent.id,
        _ => inbox,
    };
    let threads = file_all(
        &connection,
        account.id,
        &folders,
        &[
            "thread-archive-1-parent",
            "thread-archive-2-reply",
            "thread-sent-1-received",
            "thread-sent-2-reply",
        ],
    )
    .await;

    assert_eq!(
        threads[0], threads[1],
        "the archived reply joins its parent"
    );
    assert_eq!(
        threads[2], threads[3],
        "the sent reply joins what it answers"
    );
    assert_ne!(threads[0], threads[2]);
}

// ---------------------------------------------------------------------------
// What the subject path costs (constitution V)
// ---------------------------------------------------------------------------

/// Statements and rows to thread a "Re: Dock" reply with no references, when
/// `others` threads already share its subject, all of them inside its window.
///
/// The others are un-decorated "Dock" messages, which never use the subject
/// fallback and so each start a thread of their own.
async fn cost_of_subject_path(others: i64) -> (usize, usize, ThreadId, ThreadId) {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    install(&connection);
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let threading = ThreadingRepository::new(&connection, account.id);
    let messages = MessageRepository::new(&connection);

    let start = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap();
    let mut newest = ThreadId::new(0);
    for n in 0..others {
        let root = written(
            &messages,
            account.id,
            inbox,
            n,
            "Dock",
            start + TimeDelta::hours(n),
        )
        .await;
        newest = threading.thread(&root).await.expect("thread").thread_id;
    }
    let reply = written(
        &messages,
        account.id,
        inbox,
        others,
        "Re: Dock",
        start + TimeDelta::hours(others) + TimeDelta::days(1),
    )
    .await;
    let mut joined = ThreadId::new(0);
    let counts = counted_async(|| async {
        joined = threading.thread(&reply).await.expect("thread").thread_id;
    })
    .await;
    (counts.statements, counts.rows, newest, joined)
}

/// A message with no references, written but not yet threaded.
async fn written(
    messages: &MessageRepository<'_>,
    account: AccountId,
    inbox: MailboxId,
    n: i64,
    subject: &str,
    at: DateTime<Utc>,
) -> Message {
    let mut message = Message::new(account, inbox, at);
    message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
        "<dock-{n}@example.com>"
    )));
    message.subject = Some(subject.to_owned());
    messages.create(&mut message).await.expect("create");
    message
}

#[tokio::test]
async fn the_subject_path_costs_the_same_however_many_threads_share_the_subject() {
    let (few_statements, few_rows, _, _) = cost_of_subject_path(2).await;
    let (many_statements, many_rows, newest, joined) = cost_of_subject_path(40).await;

    assert_eq!(
        joined, newest,
        "the reply joins the newest thread in the window"
    );
    assert_eq!(few_statements, many_statements);
    assert_eq!(
        few_rows, many_rows,
        "the lookup must be bounded by the window, not by every thread with the subject"
    );
    assert_eq!(
        many_statements, BEFORE_STATEMENTS,
        "bounding the lookup must not add a statement"
    );
}

/// What threading a reply by subject cost before spec 013 bounded the lookup:
/// one statement for the subject, and the same writes as any join.
const BEFORE_STATEMENTS: usize = 4;

/// The subject lookup `LoadedIndex::load` makes, written out because the
/// point is the SQL the planner sees.
const SUBJECT_LOOKUP: &str = "SELECT id FROM threads \
     WHERE account_id = ?1 AND subject = ?2 AND last_at >= ?3 AND first_at <= ?4 AND id IS NOT ?5 \
     ORDER BY last_at DESC LIMIT 1";

#[tokio::test]
async fn the_subject_lookup_seeks_the_recent_index_with_no_sort() {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");

    let plan = test_support::plan(&connection, SUBJECT_LOOKUP).await;

    assert!(
        plan.contains("idx_threads_subject_recent"),
        "the lookup should use its own index:\n{plan}"
    );
    assert!(
        plan.contains("account_id=?") && plan.contains("subject=?") && plan.contains("last_at"),
        "the seek must bind the account, the subject and the window, or it is a scan:\n{plan}"
    );
    assert!(
        !test_support::sorts(&plan),
        "the newest-first order comes from the index, not a sort:\n{plan}"
    );
}
