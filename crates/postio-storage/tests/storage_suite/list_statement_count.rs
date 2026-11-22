use postio_model::MailboxRole;
use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
use postio_storage::seed::{seed_large, seed_small};
use postio_storage::test_support;

use postio_storage::test_support::counting::{counted_async, install};

#[tokio::test]
async fn listing_a_page_costs_the_same_statements_however_many_rows_it_returns() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");
    let connection = database.connect().await.expect("a connection");
    install(&connection);

    let page = |limit: u32| ListQuery {
        scope: ListScope::Mailbox(inbox.id),
        limit,
        after: None,
    };
    let messages = MessageRepository::new(&connection);

    // Warm first: the first `prepare` can pull schema pages in, and this is about the query's
    // shape.
    let _ = messages.page(&page(1)).await.expect("a first read");

    let mut one_row = 0;
    let one =
        counted_async(|| async { one_row = messages.page(&page(1)).await.expect("one row").len() })
            .await;

    let mut many_rows = 0;
    let many = counted_async(|| async {
        many_rows = messages.page(&page(25)).await.expect("a page").len()
    })
    .await;

    assert_eq!(one_row, 1, "a page of one should return one row");
    assert!(
        many_rows > 1,
        "a page that returned {many_rows} rows cannot demonstrate an N+1; the \
         seed did not make enough messages for this test to mean anything"
    );
    assert_eq!(
        one.statements, many.statements,
        "listing {many_rows} rows issued {} statements where one row issued \
         {}. A list that costs a statement per row is the N+1 that §18's 16ms \
         interaction budget cannot survive — and every other test in this \
         workspace still passes when it is introduced, because the rows come \
         back correct.",
        many.statements, one.statements
    );
}

/// Smaller than `threads.rs`'s hundred thousand: a count shows non-growth at ten thousand as
/// loudly, and seeds faster.
const LARGE: usize = 10_000;

#[tokio::test]
async fn a_large_mailbox_never_materialises_more_rows_than_the_page_shows() {
    // A page of fifty returns fifty rows whether windowed or read whole and sliced in Rust; the
    // rows
    // SQLite produced tell them apart (docs/PRODUCT.md §18).
    let database = test_support::temp().await;
    let report = seed_large(&database, 7, LARGE).await;
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let connection = database.connect().await.expect("a connection");
    install(&connection);
    let messages = MessageRepository::new(&connection);
    let limit = 50;
    let query = ListQuery {
        scope: ListScope::Mailbox(inbox),
        limit,
        after: None,
    };

    let mut rows = Vec::new();
    let first =
        counted_async(|| async { rows = messages.page(&query).await.expect("a first page") }).await;
    assert_eq!(
        rows.len(),
        limit as usize,
        "a page is a window, not a folder"
    );

    // A generous ceiling, since a page row may cost a few rows underneath; it rules out cost that
    // scales with the folder.
    let ceiling = limit as usize * 8;

    // The control that gives the ceiling teeth: the same repository asked for the folder, not a
    // window.
    let whole = counted_async(|| async {
        let _ = messages
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox),
                limit: LARGE as u32,
                after: None,
            })
            .await
            .expect("the folder read whole");
    })
    .await;
    assert!(
        whole.rows > ceiling,
        "reading the whole folder produced {} rows, which is inside the \
         {ceiling}-row ceiling below. Either the ceiling is now loose enough \
         to let a full mailbox read pass, or this mailbox is too small for \
         the comparison to mean anything.",
        whole.rows
    );

    assert!(
        first.rows <= ceiling,
        "showing {limit} of {LARGE} messages made SQLite produce {} rows. \
         That is the whole mailbox coming into memory to be sliced in Rust, \
         which is what §18 forbids and what page.len() cannot see.",
        first.rows
    );

    let mut cursor = rows.last().expect("a last row").cursor();
    let mut deep = first;
    for _ in 0..10 {
        deep = counted_async(|| async {
            rows = messages
                .page(&ListQuery {
                    scope: ListScope::Mailbox(inbox),
                    limit,
                    after: Some(cursor),
                })
                .await
                .expect("a deep page");
        })
        .await;
        assert_eq!(rows.len(), limit as usize);
        cursor = rows.last().expect("a last row").cursor();
    }

    assert!(
        deep.rows <= ceiling,
        "the eleventh page produced {} rows against {} for the first. Paging \
         that grows with the depth is an OFFSET scan wearing a cursor's \
         clothes, and at the bottom of a real mailbox it is the whole folder.",
        deep.rows,
        first.rows
    );
}

/// The shape of an ordinary mailbox listing, recorded before spec 003's Drafts/Outbox split lands
/// in
/// `where_clause`: statements and rows pinned on the hottest read path.
#[tokio::test]
async fn an_ordinary_listing_is_one_statement_and_no_more_rows_than_it_returns() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 40).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");
    let connection = database.connect().await.expect("a connection");
    install(&connection);

    let query = ListQuery {
        scope: ListScope::Mailbox(inbox.id),
        limit: 20,
        after: None,
    };
    let messages = MessageRepository::new(&connection);
    let _ = messages.page(&query).await.expect("a first read");

    let mut returned = 0;
    let counts =
        counted_async(|| async { returned = messages.page(&query).await.expect("a page").len() })
            .await;

    assert_eq!(returned, 20, "the seed should fill the page");
    assert_eq!(
        counts.statements, 1,
        "a page is one statement; anything more is a join or an N+1 that the \
         Drafts exclusion must not have introduced"
    );
    assert_eq!(
        counts.rows, returned,
        "the query reads exactly the rows it returns. A subquery that filtered \
         after the fact would read more than a page to fill one, which is how \
         a windowed list quietly stops being windowed"
    );
    // The planner stands in for the old `counts.nested == 0`: a scan on a windowed list is how a
    // page of 20 reads a mailbox.
    assert!(
        postio_storage::test_support::counting::scans(&connection, &messages.explain(&query),)
            .await
            .is_empty(),
        "a page of a mailbox must be a seek, not a scan"
    );
}

#[tokio::test]
async fn focus_s_snoozed_and_flagged_views_are_one_statement_and_seek() {
    let database = test_support::memory().await;
    seed_small(&database, 40).await;
    let connection = database.connect().await.expect("a connection");
    install(&connection);
    let messages = MessageRepository::new(&connection);

    for scope in [
        ListScope::Focus(postio_model::FocusScope::Snoozed),
        ListScope::Focus(postio_model::FocusScope::Flagged),
    ] {
        let query = ListQuery {
            scope,
            limit: 20,
            after: None,
        };
        let _ = messages.page(&query).await.expect("a first read");
        let counts = counted_async(|| async {
            messages.page(&query).await.expect("a page");
        })
        .await;
        assert_eq!(counts.statements, 1, "{scope:?} is one statement");
        let scans =
            postio_storage::test_support::counting::scans(&connection, &messages.explain(&query))
                .await;
        let grown: Vec<_> = scans
            .iter()
            .filter(|step| {
                postio_storage::test_support::counting::GROWING_TABLES
                    .iter()
                    .any(|table| step.contains(table))
            })
            .collect();
        assert!(grown.is_empty(), "{scope:?} scans {grown:?}; it must seek");
    }
}

#[tokio::test]
async fn naming_a_person_costs_a_page_nothing() {
    // Sender-name resolution is a key seek in the row's own sender lookup; a named sender must not
    // change a page's cost.
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");
    let connection = database.connect().await.expect("a connection");
    install(&connection);
    let query = ListQuery {
        scope: ListScope::Mailbox(inbox.id),
        limit: 25,
        after: None,
    };
    let messages = MessageRepository::new(&connection);
    let first = messages.page(&query).await.expect("a first read");
    let sender = first[0].from.clone().expect("the seed's mail has senders");

    let before = counted_async(|| async {
        messages.page(&query).await.expect("a page");
    })
    .await;
    let owner = postio_storage::repository::ContactRepository::new(&connection)
        .by_address(&sender.address)
        .await
        .expect("lookup")
        .expect("the seed records its senders");
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET name = 'Named By The User' WHERE id = ?1",
        [owner.id.get()],
    )
    .await
    .expect("name the sender");
    let mut named = Vec::new();
    let after = counted_async(|| async {
        named = messages.page(&query).await.expect("a page");
    })
    .await;

    assert_eq!(
        named[0].from.as_ref().and_then(|f| f.name.as_deref()),
        Some("Named By The User"),
        "the page shows the name, or this proves nothing"
    );
    assert_eq!(after.statements, before.statements);
    assert_eq!(after.rows, before.rows);
}
