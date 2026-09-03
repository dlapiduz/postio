//! Listing costs the same whatever the mailbox holds — counted, not timed.
//!
//! The reasoning behind counting rather than timing is in [`sql_counters`];
//! this is where the message-list path is held to it. Two budgets, both from
//! `docs/PRODUCT.md` §18: an interaction stays under 16ms, and "a mailbox is
//! never loaded into memory".

use postio_model::MailboxRole;
use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
use postio_storage::seed::{seed_large, seed_small};
use postio_storage::test_support;

use super::sql_counters::{counted, install};

#[test]
fn listing_a_page_costs_the_same_statements_however_many_rows_it_returns() {
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");
    let connection = database.connection().expect("a connection");
    install(&connection);

    let page = |limit: u32| ListQuery {
        scope: ListScope::Mailbox(inbox.id),
        limit,
        after: None,
    };
    let messages = MessageRepository::new(&connection);

    // Warm first. The first `prepare` of a statement can pull schema pages in,
    // and this is about the query's shape, not about a cold cache.
    let _ = messages.page(&page(1)).expect("a first read");

    let mut one_row = 0;
    let one = counted(|| one_row = messages.page(&page(1)).expect("one row").len());

    let mut many_rows = 0;
    let many = counted(|| many_rows = messages.page(&page(25)).expect("a page").len());

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

/// How many messages stand in for "a large mailbox" here.
///
/// `threads.rs` uses a full hundred thousand for the same shape of claim. This
/// one is deliberately smaller: what it proves is that the cost does not grow
/// with the folder, and a count says that at ten thousand exactly as loudly as
/// at a hundred thousand — while seeding ten times fewer rows keeps the suite
/// fast enough that people actually run it.
const LARGE: usize = 10_000;

#[test]
fn a_large_mailbox_never_materialises_more_rows_than_the_page_shows() {
    // §18's most-cited constraint: "Never load a whole mailbox into memory —
    // the list is windowed over paged SQLite." A page of fifty returns fifty
    // rows whether the window is real or whether the folder was read whole
    // and sliced in Rust, so `page.len()` cannot tell those apart. The number
    // of rows SQLite *produced* can, and it is the same number everywhere.
    let database = test_support::temp();
    let report = seed_large(&database, 7, LARGE);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;

    let connection = database.connection().expect("a connection");
    install(&connection);
    let messages = MessageRepository::new(&connection);
    let limit = 50;
    let query = ListQuery {
        scope: ListScope::Mailbox(inbox),
        limit,
        after: None,
    };

    let mut rows = Vec::new();
    let first = counted(|| rows = messages.page(&query).expect("a first page"));
    assert_eq!(
        rows.len(),
        limit as usize,
        "a page is a window, not a folder"
    );

    // A generous ceiling, because a row of the page may legitimately cost a
    // few rows underneath it — a join, a count, a correlated lookup. What it
    // rules out is the only failure that matters: a cost that scales with the
    // folder. Reading LARGE messages to show fifty would be orders of
    // magnitude above this, not a few multiples.
    let ceiling = limit as usize * 8;

    // The control, and the reason this assertion is known to have teeth: the
    // same repository, asked for the folder instead of a window. #100 asks
    // that each counted budget "fails when the invariant it guards is
    // deliberately broken" — this demonstrates that without ever breaking the
    // code under test, and it keeps demonstrating it. Raise `ceiling` above
    // what a full read costs and this is what stops passing.
    let whole = counted(|| {
        let _ = messages
            .page(&ListQuery {
                scope: ListScope::Mailbox(inbox),
                limit: LARGE as u32,
                after: None,
            })
            .expect("the folder read whole");
    });
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

    // Ten pages in. If the window were an OFFSET scan, the rows produced would
    // climb with the depth; over an index and a cursor they do not.
    let mut cursor = rows.last().expect("a last row").cursor();
    let mut deep = first;
    for _ in 0..10 {
        deep = counted(|| {
            rows = messages
                .page(&ListQuery {
                    scope: ListScope::Mailbox(inbox),
                    limit,
                    after: Some(cursor),
                })
                .expect("a deep page");
        });
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
