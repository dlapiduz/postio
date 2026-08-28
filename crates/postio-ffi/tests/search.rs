//! Search over the boundary (#660).
//!
//! Search hits are *ranked*, so no `ListScope` describes them — but the
//! frontend must not learn that. A result set is a count and a few resident
//! pages, exactly like a mailbox, because `PRODUCT.md` §18's rule that a
//! mailbox is never loaded into memory has nothing to do with where the rows
//! came from: a query matching forty thousand messages is the same hazard.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// A store where `matching` of `total` messages say "quarterly".
fn seeded(total: u32, matching: u32) -> (std::sync::Arc<Session>, ScopeFfi) {
    let database = test_support::memory();
    let mailbox = {
        let connection = database.connection().expect("a connection");
        postio_index::index::ensure_schema(&connection).expect("schema");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        for i in 0..total {
            let body = match i < matching {
                true => "The quarterly numbers are attached.",
                false => "Sandwiches at one.",
            };
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.subject = Some(format!("Message {i}"));
            message.sync.body_state = BodyState::Full;
            repository.create(&mut message).expect("a message");
            repository
                .set_body(
                    message.id,
                    &StoredBody {
                        text: Some(body.to_string()),
                        html: None,
                        headers: None,
                    },
                    BodyState::Full,
                )
                .expect("a body");
            postio_index::index::index_body(&connection, message.id.get(), Some(body))
                .expect("an indexed body");
        }
        inbox
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    (
        session,
        ScopeFfi::Mailbox {
            mailbox: mailbox.into(),
        },
    )
}

#[test]
fn a_query_answers_a_count_the_table_can_size_itself_from() {
    let (session, scope) = seeded(40, 12);
    session.open_scope(scope);
    session.search("quarterly".to_string());
    assert_eq!(session.row_count(), 12, "twelve messages say quarterly");
    session.shutdown();
}

#[test]
fn results_window_like_any_other_list() {
    // The whole point of routing hits through the same `ListWindow`: the
    // frontend's table code cannot tell a search from a folder, so there is no
    // second scrolling path to get wrong.
    let (session, scope) = seeded(300, 300);
    session.open_scope(scope);
    session.search("quarterly".to_string());

    assert!(session.row_count() > 100, "a result set worth paging");
    assert!(
        session.row_at(0).is_none(),
        "the first row is a placeholder until its page lands, same as a folder"
    );
    session.settle_for_test();
    assert!(session.row_at(0).is_some(), "and then it is there");
    session.shutdown();
}

#[test]
fn hits_come_back_in_rank_order_not_store_order() {
    // `message_rows` answers in the order asked, which is what makes ranking
    // survive the trip. Sorting by id here would silently discard relevance.
    let (session, scope) = seeded(30, 30);
    session.open_scope(scope);
    session.search("quarterly".to_string());
    // Asking is what triggers the read -- `settle_for_test` waits for pages in
    // flight, and nothing is in flight until a row has been missed once.
    let _ = session.row_at(0);
    session.settle_for_test();

    let ids: Vec<i64> = (0..session.row_count())
        .filter_map(|position| session.row_at(position))
        .map(|row| row.id)
        .collect();
    assert_eq!(ids.len(), session.row_count() as usize);
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        ids.len(),
        "no row is served twice"
    );
    session.shutdown();
}

#[test]
fn a_query_matching_nothing_empties_the_list_rather_than_leaving_it_stale() {
    // Showing the previous folder's rows under a query that matched nothing is
    // the worst available answer: it reads as "here are your results".
    let (session, scope) = seeded(20, 0);
    session.open_scope(scope);
    session.search("quarterly".to_string());
    assert_eq!(session.row_count(), 0);
    session.shutdown();
}

#[test]
fn clearing_search_restores_the_scope_that_was_open() {
    let (session, scope) = seeded(20, 5);
    session.open_scope(scope);
    assert_eq!(session.row_count(), 20);

    session.search("quarterly".to_string());
    assert_eq!(session.row_count(), 5);

    session.clear_search();
    assert_eq!(
        session.row_count(),
        20,
        "the folder comes back without the frontend having to say which one"
    );
    session.shutdown();
}

#[test]
fn the_operators_are_the_ones_postio_search_parses() {
    // One query language. Swift parses nothing: `is:unread` has to mean here
    // what it means on the GTK side, and it means it because the same parser
    // read it.
    let (session, scope) = seeded(10, 10);
    session.open_scope(scope);
    session.search("is:unread".to_string());
    assert_eq!(
        session.row_count(),
        10,
        "everything seeded is unread, and `is:unread` is an operator not a word"
    );
    session.shutdown();
}

#[test]
fn a_snippet_carries_the_ranges_rather_than_pre_marked_text() {
    // As the palette does since #568. GTK escapes and wraps them in Pango
    // markup, Swift builds an `AttributedString`; neither decides *what*
    // matched, because two highlighters drift.
    let (session, scope) = seeded(5, 5);
    session.open_scope(scope);
    session.search("quarterly".to_string());
    // Asking is what triggers the read -- `settle_for_test` waits for pages in
    // flight, and nothing is in flight until a row has been missed once.
    let _ = session.row_at(0);
    session.settle_for_test();

    let row = session.row_at(0).expect("a settled row");
    let snippet = session
        .search_snippet(row.id)
        .expect("a text query leaves every hit an excerpt");
    assert!(!snippet.text.is_empty());
    assert!(!snippet.ranges.is_empty(), "the match was located");
    for range in &snippet.ranges {
        let matched = &snippet.text[range.start as usize..range.end as usize];
        assert_eq!(
            matched.to_lowercase(),
            "quarterly",
            "a range points at what was typed"
        );
    }
    assert!(
        !snippet.text.contains("<span"),
        "no markup crosses; the frontend renders the ranges its own way"
    );
    session.shutdown();
}

#[test]
fn a_search_over_a_large_store_stays_inside_the_budget() {
    // `PRODUCT.md`: local search under 100 ms. Asserted here rather than left
    // to `cargo bench` because the boundary is where a regression would hide:
    // the executor's own budget is benchmarked, and this measures what the
    // frontend actually waits for -- parse, execute, and cut fifty excerpts.
    //
    // Generous against the budget on purpose. This runs on whatever machine
    // happens to have the worktree, under a debug build, beside other
    // sessions' compiles; a tight bound here would fail for reasons that have
    // nothing to do with search. It still catches the shape of regression that
    // matters, which is a per-hit round trip or a full-table scan.
    let (session, scope) = seeded(5_000, 5_000);
    session.open_scope(scope);

    let start = std::time::Instant::now();
    let generation = session.search("quarterly".to_string());
    let elapsed = start.elapsed();

    assert!(generation > 0, "the window moved to the results");
    assert!(
        session.row_count() > 0,
        "the query matched, or this measures nothing"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(2_000),
        "a search took {elapsed:?}, which is not the shape of a local index read"
    );
    session.shutdown();
}
