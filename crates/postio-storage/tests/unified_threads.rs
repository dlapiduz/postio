//! The unified inbox groups threads across accounts at read time (#184).
//!
//! ADR 0005 Q2: a thread never spans accounts — `threads.account_id` stays
//! `NOT NULL`, threads remain per-account sync state. What the unified list
//! shows is a [`ThreadGroup`]: threads from different accounts folded into
//! one row when their JWZ roots share an `RfcMessageId`, or — roots missing
//! — when their normalised subjects match within the coalescing window. The
//! grouping is computed by the same paged query that builds the list, and
//! the copies both stay: dedupe is display-only, and an action on a group
//! has every member thread to hit.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::{Message, RfcMessageId};
use postio_storage::repository::{
    MessageRepository, ThreadGroup, ThreadRepository, ThreadingRepository, UnifiedThreadListQuery,
};
use postio_storage::test_support;
use rusqlite::Connection;

fn at(hour: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 10, 0, 0, 0).unwrap() + TimeDelta::hours(hour)
}

fn file(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    hour: i64,
    rfc: Option<&str>,
    references: &[&str],
    subject: &str,
) -> (MessageId, ThreadId) {
    let mut message = Message::new(account, mailbox, at(hour));
    message.rfc_message_id = rfc.map(RfcMessageId::new);
    message.references = references.iter().map(RfcMessageId::new).collect();
    message.subject = Some(subject.to_owned());
    let id = MessageRepository::new(connection)
        .create(&mut message)
        .expect("create");
    let threaded = ThreadingRepository::new(connection, account)
        .thread(&message)
        .expect("thread");
    (id, threaded.thread_id)
}

/// Two accounts, each with an inbox.
fn two_accounts(connection: &Connection) -> ((AccountId, MailboxId), (AccountId, MailboxId)) {
    let (first, inbox) = test_support::account_with_inbox(connection);
    let mut second = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(connection)
        .create(&mut second)
        .expect("second account");
    let second_inbox = test_support::mailbox(connection, &second, "INBOX");
    ((first.id, inbox), (second.id, second_inbox.id))
}

fn page(connection: &Connection, limit: u32) -> Vec<ThreadGroup> {
    ThreadRepository::new(connection)
        .unified_page(&UnifiedThreadListQuery { limit, after: None })
        .expect("unified page")
}

#[test]
fn threads_sharing_a_root_rfc_id_group_across_accounts_and_dedupe() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection);

    // The same announcement received at both addresses, replied to in the
    // first account: three rows, two distinct messages, one conversation.
    let (_, a_thread) = file(
        &connection,
        a,
        a_inbox,
        1,
        Some("<root@example.com>"),
        &[],
        "Launch",
    );
    file(
        &connection,
        a,
        a_inbox,
        3,
        Some("<re1@example.com>"),
        &["<root@example.com>"],
        "Re: Launch",
    );
    let (_, b_thread) = file(
        &connection,
        b,
        b_inbox,
        2,
        Some("<root@example.com>"),
        &[],
        "Launch",
    );

    let groups = page(&connection, 10);
    assert_eq!(groups.len(), 1, "one conversation, however many accounts");
    let group = &groups[0];
    assert_eq!(
        group.members.len(),
        2,
        "both copies stay: an action has both threads to hit"
    );
    assert!(group.members.contains(&(a, a_thread)));
    assert!(group.members.contains(&(b, b_thread)));
    assert_eq!(
        group.row.message_count, 2,
        "dedupe is display-only, by RfcMessageId: three rows, two messages"
    );
    assert_eq!(
        group.row.last_at,
        at(3),
        "the group is as recent as its newest member"
    );
}

#[test]
fn rootless_threads_group_by_subject_within_the_window_and_not_beyond() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection);

    // No rfc ids anywhere: the subject fallback is all there is.
    file(&connection, a, a_inbox, 1, None, &[], "Sirius review");
    file(&connection, b, b_inbox, 5, None, &[], "Re: Sirius review");

    // Same subject in both accounts, but further apart than the coalescing
    // window: two unrelated conversations that happen to share four words.
    let far = 24 * postio_model::subject::COALESCING_WINDOW_DAYS + 48;
    file(&connection, a, a_inbox, 100, None, &[], "Weekly digest");
    file(
        &connection,
        b,
        b_inbox,
        100 + far,
        None,
        &[],
        "Weekly digest",
    );

    let groups = page(&connection, 10);
    let sizes: Vec<usize> = groups.iter().map(|group| group.members.len()).collect();
    assert_eq!(
        sizes,
        vec![1, 1, 2],
        "newest first: the two far-apart digests alone, then the grouped review"
    );
}

#[test]
fn a_partner_already_shown_is_never_a_second_row_across_pages() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection);

    // The grouped conversation is the newest thing in both accounts…
    file(
        &connection,
        a,
        a_inbox,
        10,
        Some("<pair@example.com>"),
        &[],
        "Paired",
    );
    file(
        &connection,
        b,
        b_inbox,
        9,
        Some("<pair@example.com>"),
        &[],
        "Paired",
    );
    // …and one older standalone per account fills the second page.
    file(
        &connection,
        a,
        a_inbox,
        2,
        Some("<solo-a@example.com>"),
        &[],
        "Alone in A",
    );
    file(
        &connection,
        b,
        b_inbox,
        1,
        Some("<solo-b@example.com>"),
        &[],
        "Alone in B",
    );

    let repository = ThreadRepository::new(&connection);
    let first = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 1,
            after: None,
        })
        .expect("first page");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].members.len(), 2, "the pair is one row");

    let second = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 10,
            after: Some(first[0].cursor()),
        })
        .expect("second page");
    let subjects: Vec<Option<&str>> = second
        .iter()
        .map(|group| group.row.subject.as_deref())
        .collect();
    assert_eq!(
        subjects,
        // Normalised, because `threads.subject` is the normalised root
        // subject — the same casing the account-scoped list rows carry.
        vec![Some("alone in a"), Some("alone in b")],
        "the absorbed partner never resurfaces as a row of its own"
    );
}
