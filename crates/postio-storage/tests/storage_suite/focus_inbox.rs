//! Focus's inbox, counted (spec 007, `contracts/engine.md`, "Store").
//!
//! Focus reads its inbox as conversations over every enabled account's
//! inbox -- the unified inbox's membership -- in at most three statements a
//! page: the window, its participants, and its markers once Focus has them.
//! The window is one statement however many accounts there are, each inbox
//! sought through its own list index and the few rows merged in that same
//! statement, so a page reads what it shows and never an inbox, let alone
//! the archive underneath it.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::{EmailAddress, Message, RfcMessageId};
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, FocusListQuery, MessageRepository, ThreadCursor, ThreadListQuery,
    ThreadListRow, ThreadRepository, ThreadingRepository,
};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, scans};

fn at(hour: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap() + TimeDelta::hours(hour)
}

/// Files a message from `sender` and threads it, as a sync pass does.
async fn file(
    connection: &Connection,
    (account, mailbox): (AccountId, MailboxId),
    hour: i64,
    rfc: &str,
    references: &[&str],
    sender: &str,
) -> (MessageId, ThreadId) {
    let mut message = Message::new(account, mailbox, at(hour));
    message.rfc_message_id = Some(RfcMessageId::new(rfc));
    message.references = references.iter().map(RfcMessageId::new).collect();
    message.subject = Some(format!("About {rfc}"));
    message.from = vec![EmailAddress::new(None::<String>, sender)];
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

/// Two accounts, each with an inbox and an archive, and a third that is
/// switched off; conversations in every one of them.
///
/// Answers the two enabled inboxes, in the order the store names them.
async fn world(connection: &Connection) -> Vec<(AccountId, MailboxId)> {
    let (ada, ada_inbox) = test_support::account_with_inbox(connection).await;
    let ada_archive = test_support::mailbox(connection, &ada, "Archive").await.id;
    let mut grace = postio_model::Account::new(
        "Second",
        EmailAddress::new(None::<String>, "grace@example.org"),
    );
    AccountRepository::new(connection)
        .create(&mut grace)
        .await
        .expect("a second account");
    let grace_inbox = test_support::mailbox(connection, &grace, "INBOX").await.id;
    let mut off =
        postio_model::Account::new("Off", EmailAddress::new(None::<String>, "off@example.net"));
    AccountRepository::new(connection)
        .create(&mut off)
        .await
        .expect("a third account");
    let off_inbox = test_support::mailbox(connection, &off, "INBOX").await.id;

    let ada_in = (ada.id, ada_inbox);
    let grace_in = (grace.id, grace_inbox);
    // A conversation of three in Ada's inbox, its newest reply archived: the
    // row is drawn from its newest member *in the inbox*.
    file(
        connection,
        ada_in,
        1,
        "<gate@example.com>",
        &[],
        "quinn@example.com",
    )
    .await;
    file(
        connection,
        ada_in,
        4,
        "<gate-2@example.com>",
        &["<gate@example.com>"],
        "tove@example.com",
    )
    .await;
    file(
        connection,
        (ada.id, ada_archive),
        9,
        "<gate-3@example.com>",
        &["<gate@example.com>", "<gate-2@example.com>"],
        "quinn@example.com",
    )
    .await;
    // One of one, in Ada's inbox.
    file(
        connection,
        ada_in,
        6,
        "<tide@example.com>",
        &[],
        "ada@example.com",
    )
    .await;
    // Archived whole: not Focus's.
    file(
        connection,
        (ada.id, ada_archive),
        8,
        "<filed@example.com>",
        &[],
        "tove@example.com",
    )
    .await;
    // A snoozed message is not a row until its time comes.
    let (snoozed, _) = file(
        connection,
        ada_in,
        7,
        "<later@example.com>",
        &[],
        "tove@example.com",
    )
    .await;
    MessageRepository::new(connection)
        .snooze(&[snoozed], Utc::now() + TimeDelta::days(1))
        .await
        .expect("snooze");
    // Grace's inbox, interleaved in time with Ada's.
    file(
        connection,
        grace_in,
        2,
        "<rota@example.org>",
        &[],
        "quinn@example.com",
    )
    .await;
    file(
        connection,
        grace_in,
        5,
        "<rota-2@example.org>",
        &["<rota@example.org>"],
        "ada@example.com",
    )
    .await;
    file(
        connection,
        grace_in,
        3,
        "<kiln@example.org>",
        &[],
        "tove@example.com",
    )
    .await;
    // The account that is off: nothing of it is listed.
    file(
        connection,
        (off.id, off_inbox),
        10,
        "<quiet@example.net>",
        &[],
        "quinn@example.com",
    )
    .await;
    AccountRepository::new(connection)
        .set_enabled(off.id, false)
        .await
        .expect("switch the third account off");

    let inboxes = ThreadRepository::new(connection)
        .unified_inboxes()
        .await
        .expect("the inboxes");
    assert_eq!(
        inboxes,
        vec![ada_in, grace_in],
        "the fixture's two enabled inboxes"
    );
    inboxes
}

fn focus(
    inboxes: &[(AccountId, MailboxId)],
    limit: u32,
    after: Option<ThreadCursor>,
) -> FocusListQuery {
    FocusListQuery {
        inboxes: inboxes.to_vec(),
        limit,
        after,
    }
}

/// The rows each inbox's own folder list shows, merged newest first: what
/// Focus's inbox is today, row for row.
async fn every_inbox_s_rows(
    connection: &Connection,
    inboxes: &[(AccountId, MailboxId)],
) -> Vec<ThreadListRow> {
    let mut rows = Vec::new();
    for (account, inbox) in inboxes {
        rows.extend(
            ThreadRepository::new(connection)
                .page(&ThreadListQuery::in_mailbox(*account, *inbox).limit(100))
                .await
                .expect("a folder page"),
        );
    }
    rows.sort_by_key(|row| std::cmp::Reverse((row.last_at, row.sort_id)));
    rows
}

#[tokio::test]
async fn focus_s_inbox_is_every_enabled_inbox_s_conversations_newest_first() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;

    let rows = ThreadRepository::new(&connection)
        .focus_page(&focus(&inboxes, 50, None))
        .await
        .expect("a Focus page");

    let expected = every_inbox_s_rows(&connection, &inboxes).await;
    assert_eq!(
        rows, expected,
        "Focus's inbox is the unified inbox's membership: each inbox's own \
         rows, representatives and participants included, merged newest first"
    );
    let subjects: Vec<_> = rows
        .iter()
        .map(|row| {
            row.latest
                .as_ref()
                .and_then(|latest| latest.subject.clone())
        })
        .collect();
    assert_eq!(
        subjects,
        [
            "About <tide@example.com>",
            "About <rota-2@example.org>",
            "About <gate-2@example.com>",
            "About <kiln@example.org>",
        ]
        .map(|subject| Some(subject.to_owned())),
        "archived, snoozed and switched-off mail is not in it, and a \
         conversation is drawn from its newest member in the inbox"
    );
}

#[tokio::test]
async fn a_focus_inbox_page_is_at_most_three_statements_and_reads_only_what_it_shows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let query = focus(&inboxes, 50, None);
    // Warm: this is about the page's shape, not a cold statement cache.
    let _ = threads.focus_page(&query).await.expect("a first read");

    let mut page = Vec::new();
    let first = counted_async(|| async {
        page = threads.focus_page(&query).await.expect("a page");
    })
    .await;
    assert!(
        page.len() > 2,
        "a page of {} rows cannot show a cost that grows with its rows",
        page.len()
    );
    assert!(
        first.statements <= 3,
        "a Focus page took {} statements; the budget is the window, its \
         participants and its markers -- never one per row or per inbox",
        first.statements
    );
    let participants: usize = page
        .iter()
        .filter(|row| row.id.is_some())
        .map(|row| row.participants.len())
        .sum();
    assert_eq!(
        first.rows,
        page.len() + participants,
        "every row the page read is a row it shows: its conversations and \
         their participants, and nothing read to be thrown away"
    );

    // A page resumed from a cursor costs the same.
    let head = threads
        .focus_page(&focus(&inboxes, 2, None))
        .await
        .expect("a head");
    let after = Some(head.last().expect("two rows").cursor());
    let resumed = counted_async(|| async {
        threads
            .focus_page(&focus(&inboxes, 2, after))
            .await
            .expect("a resumed page");
    })
    .await;
    assert!(resumed.statements <= 3, "{resumed:?}");

    // No mail table is scanned: each inbox is sought through its own list
    // index, and the only thing walked whole is the few rows those seeks
    // produced, merged.
    for (label, query) in [
        ("first", focus(&inboxes, 50, None)),
        ("resumed", focus(&inboxes, 2, after)),
    ] {
        let sql = threads.explain_focus(&query, 0);
        let scanned = scans(&connection, &sql).await;
        assert!(
            scanned
                .iter()
                .all(|step| step.starts_with("SCAN (subquery")),
            "{label}: a Focus page scans a table: {scanned:?}\n{}",
            test_support::plan(&connection, &sql).await
        );
    }
    // With one inbox there is nothing to merge: no scan and no sort at all.
    let alone = focus(&inboxes[..1], 50, None);
    let plan = test_support::plan(&connection, &threads.explain_focus(&alone, 0)).await;
    assert!(
        scans(&connection, &threads.explain_focus(&alone, 0))
            .await
            .is_empty()
            && !test_support::sorts(&plan),
        "one inbox is its own list's seek:\n{plan}"
    );
}

#[tokio::test]
async fn focus_s_inbox_pages_by_cursor_and_by_offset_to_the_same_rows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let whole = threads
        .focus_page(&focus(&inboxes, 50, None))
        .await
        .expect("the whole list");
    let ids = |rows: &[ThreadListRow]| rows.iter().map(ThreadListRow::cursor).collect::<Vec<_>>();

    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let page = threads
            .focus_page(&focus(&inboxes, 2, after))
            .await
            .expect("a page");
        let Some(last) = page.last() else { break };
        after = Some(last.cursor());
        walked.extend(page);
    }
    assert_eq!(
        ids(&walked),
        ids(&whole),
        "a cursor walk repeats and skips nothing"
    );

    let mut skipped = Vec::new();
    for offset in (0..whole.len() as u32).step_by(2) {
        skipped.extend(
            threads
                .focus_page_at(&focus(&inboxes, 2, None), offset)
                .await
                .expect("a page at an offset"),
        );
    }
    assert_eq!(
        ids(&skipped),
        ids(&whole),
        "offsets land where the cursor walk did"
    );
}

#[tokio::test]
async fn focus_s_inbox_is_counted_in_one_statement_from_an_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let rows = threads
        .focus_page(&focus(&inboxes, 50, None))
        .await
        .expect("the whole list");
    let _ = threads.focus_count(&inboxes).await.expect("warm");

    let mut total = 0;
    let counts = counted_async(|| async {
        total = threads.focus_count(&inboxes).await.expect("a count");
    })
    .await;
    assert_eq!(
        total as usize,
        rows.len(),
        "the count and the rows cannot disagree about what a row is"
    );
    assert_eq!(counts.statements, 1, "{counts:?}");
    let sql = threads.explain_focus_count(inboxes.len());
    assert!(
        scans(&connection, &sql).await.is_empty(),
        "counting Focus's inbox reads an index, never the table:\n{}",
        test_support::plan(&connection, &sql).await
    );
}
