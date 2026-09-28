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
use postio_model::listing::{InviteAnswer, MarkerKind, MarkerSummary, MarkerWhen};
use postio_model::{EmailAddress, Message, RfcMessageId};
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, DigestRepository, FocusListQuery, InviteIdentity, InviteState, Marker,
    MarkerRepository, MarkerSource, MessageRepository, ThreadCursor, ThreadGroup, ThreadListQuery,
    ThreadListRow, ThreadRepository, ThreadingRepository, UnifiedThreadListQuery,
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

/// The rows a page of groups draws.
fn rows(groups: Vec<ThreadGroup>) -> Vec<ThreadListRow> {
    groups.into_iter().map(|group| group.row).collect()
}

/// The same announcement delivered to both enabled inboxes: Ada's copy at
/// `hour`, Grace's an hour later. Answers the two copies' threads, Ada's
/// first.
async fn delivered_to_both(
    connection: &Connection,
    inboxes: &[(AccountId, MailboxId)],
    hour: i64,
) -> ((AccountId, ThreadId), (AccountId, ThreadId)) {
    let (ada, grace) = (inboxes[0], inboxes[1]);
    let (_, ada_copy) = file(
        connection,
        ada,
        hour,
        "<launch@example.net>",
        &[],
        "quinn@example.com",
    )
    .await;
    let (_, grace_copy) = file(
        connection,
        grace,
        hour + 1,
        "<launch@example.net>",
        &[],
        "quinn@example.com",
    )
    .await;
    ((ada.0, ada_copy), (grace.0, grace_copy))
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

    let rows = rows(
        ThreadRepository::new(&connection)
            .focus_page_at(&focus(&inboxes, 50, None), 0)
            .await
            .expect("a Focus page"),
    );

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

/// Three more conversations of one in Ada's inbox, and one of two, so her
/// inbox alone is a page long enough to show a cost that grows with its
/// rows.
async fn more_of_ada_s(connection: &Connection, ada: (AccountId, MailboxId)) {
    for (hour, rfc) in [
        (11, "<weir@example.com>"),
        (12, "<sluice@example.com>"),
        (13, "<lock@example.com>"),
        (14, "<dam@example.com>"),
    ] {
        file(connection, ada, hour, rfc, &[], "tove@example.com").await;
    }
    file(
        connection,
        ada,
        15,
        "<dam-2@example.com>",
        &["<dam@example.com>"],
        "quinn@example.com",
    )
    .await;
}

#[tokio::test]
async fn a_one_account_focus_page_is_at_most_three_statements_and_reads_only_what_it_shows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    // One account enabled: one inbox, and nothing any row could fold with.
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    let query = focus(alone, 50, None);
    // Warm: this is about the page's shape, not a cold statement cache.
    let _ = threads
        .focus_page_at(&query, 0)
        .await
        .expect("a first read");

    let mut page = Vec::new();
    let first = counted_async(|| async {
        page = rows(threads.focus_page_at(&query, 0).await.expect("a page"));
    })
    .await;
    assert!(
        page.len() > 2,
        "a page of {} rows cannot show a cost that grows with its rows",
        page.len()
    );
    assert!(
        first.statements <= 3,
        "a one-account Focus page took {} statements; the budget is the \
         window, its participants and its markers -- never one per row, and \
         no partner search with no other account to fold with",
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
        .focus_page_at(&focus(alone, 2, None), 0)
        .await
        .expect("a head");
    let after = Some(head.last().expect("two rows").cursor());
    let resumed = counted_async(|| async {
        threads
            .focus_page_at(&focus(alone, 2, after), 0)
            .await
            .expect("a resumed page");
    })
    .await;
    assert!(resumed.statements <= 3, "{resumed:?}");

    // One inbox is its own list's seek: no scan and no sort at all.
    for (label, query) in [
        ("first", focus(alone, 50, None)),
        ("resumed", focus(alone, 2, after)),
    ] {
        let sql = threads.explain_focus(&query, 0);
        let plan = test_support::plan(&connection, &sql).await;
        assert!(
            scans(&connection, &sql).await.is_empty() && !test_support::sorts(&plan),
            "{label}: one inbox is its own list's seek:\n{plan}"
        );
    }
}

#[tokio::test]
async fn a_focus_window_over_several_inboxes_merges_their_seeks_and_scans_no_table() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let head = threads
        .focus_page_at(&focus(&inboxes, 2, None), 0)
        .await
        .expect("a head");
    let after = Some(head.last().expect("two rows").cursor());

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
}

#[tokio::test]
async fn a_conversation_that_reached_two_inboxes_is_one_focus_row_as_it_is_in_unified() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let (ada_copy, grace_copy) = delivered_to_both(&connection, &inboxes, 11).await;
    let threads = ThreadRepository::new(&connection);

    let page = threads
        .focus_page_at(&focus(&inboxes, 50, None), 0)
        .await
        .expect("a Focus page");
    let rows_of_it: Vec<&ThreadGroup> = page
        .iter()
        .filter(|group| group.members.contains(&ada_copy) || group.members.contains(&grace_copy))
        .collect();
    assert_eq!(
        rows_of_it.len(),
        1,
        "one conversation received at two addresses is one row: {page:#?}"
    );
    let mut members = rows_of_it[0].members.clone();
    members.sort();
    let mut both = vec![ada_copy, grace_copy];
    both.sort();
    assert_eq!(
        members, both,
        "both copies stay: an action on the row reaches each account's own"
    );
    assert_eq!(
        rows_of_it[0].row.message_count, 1,
        "dedupe is display-only, by Message-ID: two rows, one message"
    );

    // Row for row, members and counts, what the unified inbox shows: the
    // same partner search, not a second one.
    let unified = threads
        .unified_page(&UnifiedThreadListQuery {
            limit: 50,
            after: None,
        })
        .await
        .expect("Unified's page");
    assert_eq!(page, unified, "Focus's inbox folds as Unified does");

    // And the count, the cursor walk and the offsets agree with the page.
    assert_eq!(
        threads.focus_count(&inboxes).await.expect("a count") as usize,
        page.len(),
        "the count and the rows cannot disagree about what a row is"
    );
    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let next = threads
            .focus_page_at(&focus(&inboxes, 2, after), 0)
            .await
            .expect("a page");
        let Some(last) = next.last() else { break };
        after = Some(last.cursor());
        walked.extend(next);
    }
    assert_eq!(walked, page, "a cursor walk repeats and skips nothing");
    let mut skipped = Vec::new();
    for offset in (0..page.len() as u32).step_by(2) {
        skipped.extend(
            threads
                .focus_page_at(&focus(&inboxes, 2, None), offset)
                .await
                .expect("a page at an offset"),
        );
    }
    assert_eq!(skipped, page, "offsets land where the cursor walk did");
}

#[tokio::test]
async fn folding_across_accounts_adds_only_the_partner_search_unified_pays() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    delivered_to_both(&connection, &inboxes, 11).await;
    let threads = ThreadRepository::new(&connection);
    let query = focus(&inboxes, 50, None);
    let _ = threads.focus_page_at(&query, 0).await.expect("warm");

    let mut page = Vec::new();
    let counts = counted_async(|| async {
        page = threads.focus_page_at(&query, 0).await.expect("a page");
    })
    .await;
    let folded = page.iter().filter(|group| group.members.len() > 1).count();
    assert_eq!(folded, 1, "the fixture folds one row");
    // The page's own three, the partner search's five -- the threads, their
    // roots, partners by root, by subject, and where those are in view --
    // and one to dedupe each folded row's counts.
    assert!(
        counts.statements <= 3 + 5 + folded,
        "a two-account Focus page took {} statements: {counts:?}",
        counts.statements
    );
}

#[tokio::test]
async fn focus_s_inbox_pages_by_cursor_and_by_offset_to_the_same_rows() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let whole = threads
        .focus_page_at(&focus(&inboxes, 50, None), 0)
        .await
        .expect("the whole list");
    let ids = |rows: &[ThreadGroup]| rows.iter().map(ThreadGroup::cursor).collect::<Vec<_>>();

    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let page = threads
            .focus_page_at(&focus(&inboxes, 2, after), 0)
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
    for inboxes in [&inboxes[..1], &inboxes[..]] {
        let rows = threads
            .focus_page_at(&focus(inboxes, 50, None), 0)
            .await
            .expect("the whole list");
        let _ = threads.focus_count(inboxes).await.expect("warm");

        let mut total = 0;
        let counts = counted_async(|| async {
            total = threads.focus_count(inboxes).await.expect("a count");
        })
        .await;
        assert_eq!(
            total as usize,
            rows.len(),
            "the count and the rows cannot disagree about what a row is"
        );
        if inboxes.len() == 1 {
            assert_eq!(counts.statements, 1, "{counts:?}");
        }
    }
    let sql = threads.explain_focus_count(inboxes.len());
    assert!(
        scans(&connection, &sql).await.is_empty(),
        "counting Focus's inbox reads an index, never the table:\n{}",
        test_support::plan(&connection, &sql).await
    );
}

/// The message filed as `rfc`.
async fn id_of(connection: &Connection, rfc: &str) -> MessageId {
    MessageId::new(
        postio_storage::sql::one(
            connection,
            "SELECT id FROM messages WHERE rfc_message_id = ?1",
            [rfc],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("the message"),
    )
}

fn marker(message: MessageId, kind: MarkerKind, excerpt: Option<&str>) -> Marker {
    Marker {
        message,
        kind,
        source: MarkerSource::Detector,
        span: excerpt.map(|excerpt| (0, excerpt.chars().count() as u32)),
        excerpt: excerpt.map(str::to_owned),
        starts_at: None,
        ends_at: None,
        due_at: None,
        invite: None,
        invite_state: None,
        answer: None,
        dismissed_at: None,
    }
}

/// Markers on Ada's inbox: a question on the *older* message of the gate
/// conversation, a to-do and a dismissed one on two conversations of one,
/// a cancelled invitation someone had accepted on a third, and one on each
/// message of the dam conversation, whose newer one the row draws.
async fn mark_ada_s(connection: &Connection) -> Vec<(&'static str, Option<MarkerSummary>)> {
    let markers = MarkerRepository::new(connection);
    markers
        .insert(&marker(
            id_of(connection, "<dam@example.com>").await,
            MarkerKind::Question,
            Some("Is the dam survey still on?"),
        ))
        .await
        .expect("a question");
    markers
        .insert(&marker(
            id_of(connection, "<dam-2@example.com>").await,
            MarkerKind::Todo,
            Some("Book the survey boat."),
        ))
        .await
        .expect("a to-do");
    let asked = "Can the gate open by nine?";
    markers
        .insert(&marker(
            id_of(connection, "<gate@example.com>").await,
            MarkerKind::Question,
            Some(asked),
        ))
        .await
        .expect("a question");
    let due = at(40);
    let mut todo = marker(
        id_of(connection, "<weir@example.com>").await,
        MarkerKind::Todo,
        Some("Send the weir readings."),
    );
    todo.due_at = Some(due);
    markers.insert(&todo).await.expect("a to-do");
    let dismissed = id_of(connection, "<sluice@example.com>").await;
    markers
        .insert(&marker(dismissed, MarkerKind::Question, Some("Any news?")))
        .await
        .expect("a question");
    markers
        .dismiss(dismissed, Some(at(20)))
        .await
        .expect("dismissed");
    let (starts_at, ends_at) = (at(50), at(51));
    markers
        .insert(&Marker {
            source: MarkerSource::Calendar,
            span: None,
            excerpt: None,
            starts_at: Some(starts_at),
            ends_at: Some(ends_at),
            invite: Some(InviteIdentity {
                uid: "lock-inspection@calendar.example".to_owned(),
                sequence: 2,
                stamp: None,
            }),
            invite_state: Some(InviteState::Cancelled),
            answer: Some(InviteAnswer::Accepted),
            ..marker(
                id_of(connection, "<lock@example.com>").await,
                MarkerKind::Invite,
                None,
            )
        })
        .await
        .expect("an invitation");
    vec![
        (
            "About <gate-2@example.com>",
            Some(MarkerSummary {
                kind: MarkerKind::Question,
                when: None,
                excerpt: Some(asked.to_owned()),
                answer: None,
                cancelled: false,
            }),
        ),
        (
            "About <weir@example.com>",
            Some(MarkerSummary {
                kind: MarkerKind::Todo,
                when: Some(MarkerWhen::Due(due)),
                excerpt: Some("Send the weir readings.".to_owned()),
                answer: None,
                cancelled: false,
            }),
        ),
        ("About <sluice@example.com>", None),
        (
            "About <lock@example.com>",
            Some(MarkerSummary {
                kind: MarkerKind::Invite,
                when: Some(MarkerWhen::Event { starts_at, ends_at }),
                excerpt: None,
                answer: Some(InviteAnswer::Accepted),
                cancelled: true,
            }),
        ),
        ("About <tide@example.com>", None),
        (
            "About <dam-2@example.com>",
            Some(MarkerSummary {
                kind: MarkerKind::Todo,
                when: None,
                excerpt: Some("Book the survey boat.".to_owned()),
                answer: None,
                cancelled: false,
            }),
        ),
    ]
}

fn subject(row: &ThreadListRow) -> Option<&str> {
    row.latest
        .as_ref()
        .and_then(|latest| latest.subject.as_deref())
}

#[tokio::test]
async fn a_focus_page_draws_each_conversation_s_marker_in_its_budget_of_three() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    let expected = mark_ada_s(&connection).await;
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    let query = focus(alone, 50, None);
    let _ = threads.focus_page_at(&query, 0).await.expect("warm");

    let mut page = Vec::new();
    let counts = counted_async(|| async {
        page = rows(threads.focus_page_at(&query, 0).await.expect("a page"));
    })
    .await;
    for (drawn, marker) in &expected {
        let row = page
            .iter()
            .find(|row| subject(row) == Some(drawn))
            .unwrap_or_else(|| panic!("{drawn} is listed"));
        assert_eq!(&row.marker, marker, "{drawn}");
    }
    assert!(
        counts.statements <= 3,
        "a Focus page with markers took {} statements; the budget is the \
         window, its participants and its markers",
        counts.statements
    );
    let participants: usize = page
        .iter()
        .filter(|row| row.id.is_some())
        .map(|row| row.participants.len())
        .sum();
    let marked = page.iter().filter(|row| row.marker.is_some()).count();
    assert_eq!(
        counts.rows,
        page.len() + participants + marked,
        "one marker read per row that draws one, and nothing read to be \
         thrown away"
    );
    let ids: Vec<ThreadId> = page.iter().filter_map(|row| row.id).collect();
    let sql = threads.explain_focus_markers(alone.len(), ids.len(), 0);
    let plan = test_support::plan(&connection, &sql).await;
    assert!(
        scans(&connection, &sql).await.is_empty() && plan.contains("(thread_id=?)"),
        "reading a page's markers seeks the page's conversations and scans \
         nothing:\n{plan}"
    );
}

#[tokio::test]
async fn markers_are_focus_s_and_a_classic_list_neither_reads_nor_draws_them() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    let (ada, ada_inbox) = inboxes[0];
    let threads = ThreadRepository::new(&connection);
    let folder = ThreadListQuery::in_mailbox(ada, ada_inbox).limit(50);
    let unified = UnifiedThreadListQuery {
        limit: 50,
        after: None,
    };
    let _ = threads.page(&folder).await.expect("warm");
    let _ = threads.unified_page(&unified).await.expect("warm");
    let folder_before = counted_async(|| async {
        threads.page(&folder).await.expect("a folder page");
    })
    .await;
    let unified_before = counted_async(|| async {
        threads
            .unified_page(&unified)
            .await
            .expect("a unified page");
    })
    .await;

    mark_ada_s(&connection).await;

    let mut folder_rows = Vec::new();
    let folder_after = counted_async(|| async {
        folder_rows = threads.page(&folder).await.expect("a folder page");
    })
    .await;
    let mut unified_rows = Vec::new();
    let unified_after = counted_async(|| async {
        unified_rows = rows(threads.unified_page(&unified).await.expect("a page"));
    })
    .await;
    assert_eq!(
        folder_after, folder_before,
        "the folder's page costs what it did"
    );
    assert_eq!(unified_after, unified_before, "and so does Unified's");
    assert!(
        folder_rows
            .iter()
            .chain(&unified_rows)
            .all(|row| row.marker.is_none()),
        "the classic lists draw no marker"
    );
    let focus = rows(
        threads
            .focus_page_at(&focus(&inboxes[..1], 50, None), 0)
            .await
            .expect("a Focus page"),
    );
    assert_eq!(
        focus.iter().filter(|row| row.marker.is_some()).count(),
        4,
        "while Focus's page of the same inbox draws its four"
    );
}

/// Every row of Focus's inbox over `inboxes`, three ways -- one page, a
/// cursor walk, and offsets -- after checking the three and the count agree.
async fn agreed(
    threads: &ThreadRepository<'_>,
    inboxes: &[(AccountId, MailboxId)],
) -> Vec<ThreadListRow> {
    let whole = threads
        .focus_page_at(&focus(inboxes, 50, None), 0)
        .await
        .expect("the whole list");
    assert_eq!(
        threads.focus_count(inboxes).await.expect("a count") as usize,
        whole.len(),
        "the count and the rows cannot disagree about what a row is"
    );
    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let page = threads
            .focus_page_at(&focus(inboxes, 2, after), 0)
            .await
            .expect("a page");
        let Some(last) = page.last() else { break };
        after = Some(last.cursor());
        walked.extend(page);
    }
    assert_eq!(walked, whole, "a cursor walk repeats and skips nothing");
    let mut skipped = Vec::new();
    for offset in (0..whole.len() as u32).step_by(2) {
        skipped.extend(
            threads
                .focus_page_at(&focus(inboxes, 2, None), offset)
                .await
                .expect("a page at an offset"),
        );
    }
    assert_eq!(skipped, whole, "offsets land where the cursor walk did");
    rows(whole)
}

fn subjects(rows: &[ThreadListRow]) -> Vec<&str> {
    rows.iter().filter_map(subject).collect()
}

#[tokio::test]
async fn held_mail_leaves_focus_s_inbox_at_every_membership_site() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    mark_ada_s(&connection).await;
    let threads = ThreadRepository::new(&connection);
    let digests = DigestRepository::new(&connection);
    // A conversation of one, held whole; and the newest message of the dam
    // conversation, which leaves its older message to draw the row.
    for rfc in ["<weir@example.com>", "<dam-2@example.com>"] {
        digests
            .hold(id_of(&connection, rfc).await, "Newsletters", at(20))
            .await
            .expect("held");
    }

    for inboxes in [&inboxes[..1], &inboxes[..]] {
        let rows = agreed(&threads, inboxes).await;
        let shown = subjects(&rows);
        assert!(
            !shown.contains(&"About <weir@example.com>"),
            "a held conversation is not a row: {shown:?}"
        );
        assert!(
            !shown.contains(&"About <dam-2@example.com>"),
            "a held message draws no row: {shown:?}"
        );
        let dam = rows
            .iter()
            .find(|row| subject(row) == Some("About <dam@example.com>"))
            .expect("the dam conversation is drawn from what is not held");
        assert_eq!(dam.unread_count, 1, "a held message is not unread here");
        assert_eq!(
            dam.message_count, 2,
            "the badge is still the conversation's size"
        );
        assert_eq!(
            dam.marker.as_ref().map(|marker| marker.kind),
            Some(MarkerKind::Question),
            "a held message's marker is not the row's"
        );
    }

    // Holding is Focus's: the classic inbox and Unified still list it all.
    let (ada, ada_inbox) = inboxes[0];
    let folder = threads
        .page(&ThreadListQuery::in_mailbox(ada, ada_inbox).limit(50))
        .await
        .expect("the folder's rows");
    assert!(subjects(&folder).contains(&"About <weir@example.com>"));
    assert!(subjects(&folder).contains(&"About <dam-2@example.com>"));
    let unified = rows(
        threads
            .unified_page(&UnifiedThreadListQuery {
                limit: 50,
                after: None,
            })
            .await
            .expect("Unified's rows"),
    );
    assert!(subjects(&unified).contains(&"About <weir@example.com>"));
}

#[tokio::test]
async fn held_mail_rejoins_when_released_or_when_its_delivery_is_archived() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    let digests = DigestRepository::new(&connection);
    let before = agreed(&threads, alone).await.len();
    let (weir, sluice, lock) = (
        id_of(&connection, "<weir@example.com>").await,
        id_of(&connection, "<sluice@example.com>").await,
        id_of(&connection, "<lock@example.com>").await,
    );
    for held in [weir, sluice, lock] {
        digests
            .hold(held, "Newsletters", at(20))
            .await
            .expect("held");
    }
    assert_eq!(agreed(&threads, alone).await.len(), before - 3);

    // Delivered, the mail is the digest's row, not three of the inbox's.
    let delivery = digests
        .deliver("Newsletters", at(30), at(30))
        .await
        .expect("a delivery")
        .expect("it held something");
    assert_eq!(agreed(&threads, alone).await.len(), before - 3);

    // Released -- its sender stopped, or its rule removed -- it rejoins.
    digests.release(lock).await.expect("released");
    let rows = agreed(&threads, alone).await;
    assert_eq!(rows.len(), before - 2);
    assert!(subjects(&rows).contains(&"About <lock@example.com>"));

    // The delivery archived, its holds no longer keep anything out.
    digests
        .archive_delivery(delivery, at(31))
        .await
        .expect("archived");
    let rows = agreed(&threads, alone).await;
    assert_eq!(rows.len(), before);
    assert!(subjects(&rows).contains(&"About <weir@example.com>"));
}

#[tokio::test]
async fn leaving_held_mail_out_costs_no_statement_and_scans_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    mark_ada_s(&connection).await;
    DigestRepository::new(&connection)
        .hold(
            id_of(&connection, "<weir@example.com>").await,
            "Newsletters",
            at(20),
        )
        .await
        .expect("held");
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    let query = focus(alone, 50, None);
    let _ = threads.focus_page_at(&query, 0).await.expect("warm");

    let mut page = Vec::new();
    let counts = counted_async(|| async {
        page = rows(threads.focus_page_at(&query, 0).await.expect("a page"));
    })
    .await;
    assert!(counts.statements <= 3, "{counts:?}");
    let participants: usize = page
        .iter()
        .filter(|row| row.id.is_some())
        .map(|row| row.participants.len())
        .sum();
    let marked = page.iter().filter(|row| row.marker.is_some()).count();
    assert_eq!(
        counts.rows,
        page.len() + participants + marked,
        "{counts:?}"
    );

    let ids: Vec<ThreadId> = page.iter().filter_map(|row| row.id).collect();
    for (label, sql) in [
        ("the window", threads.explain_focus(&query, 0)),
        ("the count", threads.explain_focus_count(1)),
        (
            "the markers",
            threads.explain_focus_markers(1, ids.len(), 0),
        ),
    ] {
        let plan = test_support::plan(&connection, &sql).await;
        assert!(
            scans(&connection, &sql).await.is_empty(),
            "{label} scans a table to leave held mail out:\n{plan}"
        );
    }
    let plan = test_support::plan(&connection, &threads.explain_focus(&query, 0)).await;
    assert!(
        !test_support::sorts(&plan),
        "one inbox never sorts:\n{plan}"
    );
}

#[tokio::test]
async fn which_conversations_draw_a_marker_is_one_statement_sought_from_the_markers() {
    // The has-action filter (spec 007 US1 scenario 5, T048): its rows are
    // the Focus rows that draw a marker, and which those are is read from
    // the markers -- a few hundred -- rather than by walking an inbox of a
    // hundred thousand to ask each row. `idx_markers_open` was planned as a
    // partial index, which this engine's planner does not read.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    mark_ada_s(&connection).await;
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    let _ = threads.focus_marked(alone).await.expect("warm");

    let mut marked = None;
    let counts = counted_async(|| async {
        marked = Some(threads.focus_marked(alone).await.expect("the marked"));
    })
    .await;
    let marked = marked.expect("read");
    assert_eq!(counts.statements, 1, "{counts:?}");
    assert_eq!(
        marked.len(),
        4,
        "dam, lock, weir and gate draw markers; sluice's was dismissed: {marked:?}"
    );
    assert_eq!(
        counts.rows, 4,
        "a row read per conversation, none thrown away"
    );
    let sql = threads.explain_focus_marked(alone.len());
    assert!(
        scans(&connection, &sql).await.is_empty(),
        "the marked conversations are sought from the markers, never a walk:\n{}",
        test_support::plan(&connection, &sql).await
    );
}

#[tokio::test]
async fn a_has_action_page_is_the_marked_rows_newest_first_in_three_statements() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    more_of_ada_s(&connection, inboxes[0]).await;
    let expected = mark_ada_s(&connection).await;
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    let marked = threads.focus_marked(alone).await.expect("the marked");
    let _ = threads
        .focus_marked_page(alone, &marked, 0, 50)
        .await
        .expect("warm");

    let mut page = Vec::new();
    let counts = counted_async(|| async {
        page = rows(
            threads
                .focus_marked_page(alone, &marked, 0, 50)
                .await
                .expect("a page"),
        );
    })
    .await;
    let subjects: Vec<_> = page.iter().filter_map(subject).collect();
    assert_eq!(
        subjects,
        [
            "About <dam-2@example.com>",
            "About <lock@example.com>",
            "About <weir@example.com>",
            "About <gate-2@example.com>",
        ],
        "the rows that draw a marker, newest first, each drawn from its \
         newest message in the inbox as the inbox draws it"
    );
    for row in &page {
        let drawn = subject(row).expect("a subject");
        let wanted = expected
            .iter()
            .find(|(subject, _)| *subject == drawn)
            .map(|(_, marker)| marker.clone())
            .expect("a row the markers named");
        assert_eq!(row.marker, wanted, "{drawn} draws its marker");
    }
    assert!(
        counts.statements <= 3,
        "a has-action page is the window, its participants and its markers: {counts:?}"
    );
    let second = rows(
        threads
            .focus_marked_page(alone, &marked, 2, 50)
            .await
            .expect("a later page"),
    );
    assert_eq!(
        second.iter().filter_map(subject).collect::<Vec<_>>(),
        ["About <weir@example.com>", "About <gate-2@example.com>"],
        "a page by offset"
    );
    let sql = threads.explain_focus_marked_page(
        alone.len(),
        marked.threads.len(),
        marked.lone.len(),
        0,
        50,
    );
    let scanned = scans(&connection, &sql).await;
    assert!(
        scanned.is_empty(),
        "the page seeks the marked conversations, never the inbox: {scanned:?}\n{}",
        test_support::plan(&connection, &sql).await
    );
}

#[tokio::test]
async fn focus_s_unread_count_is_its_unread_conversations_in_one_statement() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let inboxes = world(&connection).await;
    let alone = &inboxes[..1];
    let threads = ThreadRepository::new(&connection);
    // Every fixture message arrives unread: gate and tide are Ada's two
    // conversations, and the snoozed one is not a row.
    let mut unread = 0;
    let counts = counted_async(|| async {
        unread = threads.focus_unread(alone).await.expect("the count");
    })
    .await;
    assert_eq!(unread, 2);
    assert_eq!(counts.statements, 1, "{counts:?}");
    let sql = threads.explain_focus_unread(alone.len());
    assert!(
        scans(&connection, &sql).await.is_empty(),
        "sought through an inbox index:\n{}",
        test_support::plan(&connection, &sql).await
    );
}
