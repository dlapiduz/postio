//! A thread never spans accounts (ADR 0005 Q2); the unified list folds threads from different
//! accounts into one row when their roots share an `RfcMessageId` or, roots missing, their subjects
//! match within the coalescing window. Dedupe is display-only, and the view is the inboxes (#1692).

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::{Message, RfcMessageId};
use postio_storage::Connection;
use postio_storage::repository::{
    MessageRepository, ThreadGroup, ThreadRepository, ThreadingRepository, UnifiedThreadListQuery,
};
use postio_storage::test_support;

fn at(hour: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 10, 0, 0, 0).unwrap() + TimeDelta::hours(hour)
}

async fn file(
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
        .await
        .expect("create");
    let threaded = ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("thread");
    (id, threaded.thread_id)
}

async fn two_accounts(connection: &Connection) -> ((AccountId, MailboxId), (AccountId, MailboxId)) {
    let (first, inbox) = test_support::account_with_inbox(connection).await;
    let mut second = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(connection)
        .create(&mut second)
        .await
        .expect("second account");
    let second_inbox = test_support::mailbox(connection, &second, "INBOX").await;
    ((first.id, inbox), (second.id, second_inbox.id))
}

async fn page(connection: &Connection, limit: u32) -> Vec<ThreadGroup> {
    ThreadRepository::new(connection)
        .unified_page(&UnifiedThreadListQuery { limit, after: None })
        .await
        .expect("unified page")
}

#[tokio::test]
async fn threads_sharing_a_root_rfc_id_group_across_accounts_and_dedupe() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;

    let (_, a_thread) = file(
        &connection,
        a,
        a_inbox,
        1,
        Some("<root@example.com>"),
        &[],
        "Launch",
    )
    .await;
    file(
        &connection,
        a,
        a_inbox,
        3,
        Some("<re1@example.com>"),
        &["<root@example.com>"],
        "Re: Launch",
    )
    .await;
    let (_, b_thread) = file(
        &connection,
        b,
        b_inbox,
        2,
        Some("<root@example.com>"),
        &[],
        "Launch",
    )
    .await;

    let groups = page(&connection, 10).await;
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

#[tokio::test]
async fn rootless_threads_group_by_subject_within_the_window_and_not_beyond() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;

    file(&connection, a, a_inbox, 1, None, &[], "Sirius review").await;
    file(&connection, b, b_inbox, 5, None, &[], "Re: Sirius review").await;

    let far = 24 * postio_model::subject::COALESCING_WINDOW_DAYS + 48;
    file(&connection, a, a_inbox, 100, None, &[], "Weekly digest").await;
    file(
        &connection,
        b,
        b_inbox,
        100 + far,
        None,
        &[],
        "Weekly digest",
    )
    .await;

    let groups = page(&connection, 10).await;
    let sizes: Vec<usize> = groups.iter().map(|group| group.members.len()).collect();
    assert_eq!(
        sizes,
        vec![1, 1, 2],
        "newest first: the two far-apart digests alone, then the grouped review"
    );
}

#[tokio::test]
async fn a_partner_already_shown_is_never_a_second_row_across_pages() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;

    file(
        &connection,
        a,
        a_inbox,
        10,
        Some("<pair@example.com>"),
        &[],
        "Paired",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        9,
        Some("<pair@example.com>"),
        &[],
        "Paired",
    )
    .await;
    file(
        &connection,
        a,
        a_inbox,
        2,
        Some("<solo-a@example.com>"),
        &[],
        "Alone in A",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        1,
        Some("<solo-b@example.com>"),
        &[],
        "Alone in B",
    )
    .await;

    let repository = ThreadRepository::new(&connection);
    let first = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 1,
            after: None,
        })
        .await
        .expect("first page");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].members.len(), 2, "the pair is one row");

    let second = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 10,
            after: Some(first[0].cursor()),
        })
        .await
        .expect("second page");
    let subjects: Vec<Option<&str>> = second
        .iter()
        .map(|group| group.row.subject.as_deref())
        .collect();
    assert_eq!(
        subjects,
        vec![Some("Alone in A"), Some("Alone in B")],
        "the absorbed partner never resurfaces as a row of its own"
    );
}

/// The total must equal the rows the walk produces, so it cannot be a thread count: a grouped pair
/// is two threads and one row. Asserted against the walk, not a literal.
#[tokio::test]
async fn the_group_count_is_what_walking_every_page_produces() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;

    file(
        &connection,
        a,
        a_inbox,
        20,
        Some("<r@example.com>"),
        &[],
        "Root pair",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        19,
        Some("<r@example.com>"),
        &[],
        "Root pair",
    )
    .await;

    file(&connection, a, a_inbox, 16, None, &[], "Subject pair").await;
    file(&connection, b, b_inbox, 15, None, &[], "Re: Subject pair").await;

    let far = 24 * postio_model::subject::COALESCING_WINDOW_DAYS + 48;
    file(&connection, a, a_inbox, 200, None, &[], "Weekly digest").await;
    file(
        &connection,
        b,
        b_inbox,
        200 + far,
        None,
        &[],
        "Weekly digest",
    )
    .await;

    file(&connection, a, a_inbox, 30, None, &[], "Same account twice").await;
    file(&connection, a, a_inbox, 31, None, &[], "Same account twice").await;

    file(
        &connection,
        a,
        a_inbox,
        5,
        Some("<solo-a@example.com>"),
        &[],
        "Alone in A",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        4,
        Some("<solo-b@example.com>"),
        &[],
        "Alone in B",
    )
    .await;

    let repository = ThreadRepository::new(&connection);

    let mut walked = 0usize;
    let mut after = None;
    loop {
        let groups = repository
            .unified_page(&UnifiedThreadListQuery { limit: 2, after })
            .await
            .expect("unified page");
        let Some(last) = groups.last() else { break };
        after = Some(last.cursor());
        walked += groups.len();
    }

    assert_eq!(
        walked, 8,
        "the walk folds exactly the two cross-account pairs"
    );
    assert_eq!(
        repository.unified_count().await.expect("unified count") as usize,
        walked,
        "the count and the walk have to agree about what a row is -- a list \
         told there are more rows than the pages can fill ends in trailing \
         placeholders that never resolve"
    );
}

/// The offset is counted from the cursor every time, which is why `postio_runtime::store` keeps
/// seek marks.
#[tokio::test]
async fn an_offset_window_is_the_walk_from_that_row_on() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;

    for hour in [1, 3, 5, 7] {
        file(
            &connection,
            a,
            a_inbox,
            hour,
            None,
            &[],
            &format!("Note {hour}"),
        )
        .await;
    }
    file(
        &connection,
        a,
        a_inbox,
        9,
        Some("<p@example.com>"),
        &[],
        "Paired",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        8,
        Some("<p@example.com>"),
        &[],
        "Paired",
    )
    .await;
    file(&connection, b, b_inbox, 2, None, &[], "Only in B").await;

    let repository = ThreadRepository::new(&connection);
    let all = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 50,
            after: None,
        })
        .await
        .expect("the whole list");
    assert_eq!(all.len(), 6, "the pair is one row");

    for offset in 0..all.len() as u32 {
        let window = repository
            .unified_page_at(
                &UnifiedThreadListQuery {
                    limit: 2,
                    after: None,
                },
                offset,
            )
            .await
            .expect("offset window");
        let expected: Vec<Option<&str>> = all[offset as usize..]
            .iter()
            .take(2)
            .map(|group| group.row.subject.as_deref())
            .collect();
        let actual: Vec<Option<&str>> = window
            .iter()
            .map(|group| group.row.subject.as_deref())
            .collect();
        assert_eq!(actual, expected, "the window at row {offset}");
    }
}

/// A disabled account drops out silently (ADR 0005 Q10), in the partner search too: a disabled
/// thread that absorbed an enabled partner would fold a visible row into one the user cannot see.
#[tokio::test]
async fn a_disabled_account_is_not_in_the_unified_view_at_all() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;

    file(
        &connection,
        a,
        a_inbox,
        1,
        Some("<solo-a@example.com>"),
        &[],
        "Only in A",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        2,
        Some("<solo-b@example.com>"),
        &[],
        "Only in B",
    )
    .await;
    file(
        &connection,
        a,
        a_inbox,
        3,
        Some("<pair@example.com>"),
        &[],
        "Shared",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        9,
        Some("<pair@example.com>"),
        &[],
        "Shared",
    )
    .await;

    let repository = ThreadRepository::new(&connection);
    assert_eq!(
        repository.unified_count().await.expect("count"),
        3,
        "with both accounts enabled: two solos and the shared pair"
    );

    postio_storage::repository::AccountRepository::new(&connection)
        .set_enabled(b, false)
        .await
        .expect("disable the second account");

    let rows: Vec<Option<String>> = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 10,
            after: None,
        })
        .await
        .expect("unified page")
        .into_iter()
        .map(|group| group.row.subject)
        .collect();
    assert_eq!(
        rows,
        vec![Some("Shared".to_owned()), Some("Only in A".to_owned())],
        "the disabled account's own conversation is gone, and the shared one \
         is still drawn from the account that is still enabled"
    );
    assert_eq!(
        repository.unified_count().await.expect("count"),
        2,
        "the count agrees with the rows, or the list grows placeholders"
    );
}

/// Unified is every enabled account's inbox, not all its mail, so a filed-away conversation leaves
/// the view as in a folder (#1692).
#[tokio::test]
async fn unified_lists_the_inboxes_of_every_account_and_nothing_filed_away() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), (b, b_inbox)) = two_accounts(&connection).await;
    let a_archive = test_support::mailbox(
        &connection,
        &postio_storage::repository::AccountRepository::new(&connection)
            .get(a)
            .await
            .expect("read the account")
            .expect("the account"),
        "Archive",
    )
    .await
    .id;

    file(
        &connection,
        a,
        a_inbox,
        1,
        Some("<a1@example.com>"),
        &[],
        "In A's inbox",
    )
    .await;
    file(
        &connection,
        b,
        b_inbox,
        2,
        Some("<b1@example.com>"),
        &[],
        "In B's inbox",
    )
    .await;
    file(
        &connection,
        a,
        a_archive,
        5,
        Some("<old@example.com>"),
        &[],
        "Long filed",
    )
    .await;
    let (archived, _) = file(
        &connection,
        b,
        b_inbox,
        4,
        Some("<b2@example.com>"),
        &[],
        "Archived from B",
    )
    .await;
    MessageRepository::new(&connection)
        .move_to(&[archived], a_archive)
        .await
        .expect("archive it");

    let repository = ThreadRepository::new(&connection);
    let subjects: Vec<String> = repository
        .unified_page(&UnifiedThreadListQuery {
            limit: 10,
            after: None,
        })
        .await
        .expect("unified page")
        .into_iter()
        .filter_map(|group| group.row.latest.and_then(|latest| latest.subject))
        .collect();
    assert_eq!(
        subjects,
        vec!["In B's inbox".to_owned(), "In A's inbox".to_owned()],
        "both inboxes, newest first, and nothing that has been filed away"
    );

    let inboxes = repository
        .count_of(&postio_storage::repository::ThreadListQuery::in_mailbox(
            a, a_inbox,
        ))
        .await
        .expect("A's inbox count")
        + repository
            .count_of(&postio_storage::repository::ThreadListQuery::in_mailbox(
                b, b_inbox,
            ))
            .await
            .expect("B's inbox count");
    assert_eq!(inboxes, 2);
    assert_eq!(
        repository.unified_count().await.expect("unified count"),
        inboxes,
        "the unified count is the inboxes' counts"
    );
}

/// A scan of `messages` or `threads` here is how "only the inboxes" would quietly come to read
/// every message.
#[tokio::test]
async fn the_unified_page_seeks_each_inbox_and_scans_no_mail() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ((a, a_inbox), _) = two_accounts(&connection).await;
    let repository = ThreadRepository::new(&connection);

    let finding = postio_storage::test_support::counting::scans(
        &connection,
        &repository.explain_unified_inboxes(),
    )
    .await;
    assert!(
        finding
            .iter()
            .all(|step| !step.contains("messages") && !step.contains("threads")),
        "finding the inboxes must not read mail: {finding:?}"
    );

    for query in [
        postio_storage::repository::ThreadListQuery::in_mailbox(a, a_inbox),
        postio_storage::repository::ThreadListQuery::in_mailbox(a, a_inbox).after(
            postio_storage::repository::ThreadCursor {
                last_at: at(5),
                id: 9,
            },
        ),
    ] {
        let scanned =
            postio_storage::test_support::counting::scans(&connection, &repository.explain(&query))
                .await;
        assert!(
            scanned.is_empty(),
            "each inbox's window is a seek, not a scan: {scanned:?}"
        );
    }
}
