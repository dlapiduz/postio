//! Search, windowed like any other list.
//!
//! `PRODUCT.md` §1 puts finding things among the three jobs Postio must beat
//! the alternatives at, so a macOS build without search is missing a third of
//! the product rather than a feature.
//!
//! What these guard is that it is *the same* search. One query language, one
//! hit limit, one excerpt rule, one answer about what matched — the frontend
//! gets results, never the job of producing them.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// A store with three messages whose bodies are indexed, and its inbox.
fn searchable() -> (std::sync::Arc<Session>, ScopeFfi) {
    let database = test_support::memory();
    let mailbox = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        // The FTS tables are the index's, created on demand rather than by a
        // store migration -- the body index stores no content (#407) and lives
        // beside the store rather than in it.
        postio_index::index::ensure_schema(&connection).expect("the index schema");
        let repository = MessageRepository::new(&connection);
        for (subject, body) in [
            ("Quarterly figures", "the quarterly numbers we discussed"),
            ("Lunch", "quarterly is not what this is about, lunch is"),
            ("Roadmap", "nothing in here says that word"),
        ] {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.subject = Some(subject.to_string());
            message.sync.body_state = BodyState::Full;
            repository.create(&mut message).expect("a message");
            repository
                .set_body(
                    message.id,
                    &StoredBody {
                        text: Some(body.to_string()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
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
    let scope = ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    };
    session.open_scope(scope.clone());
    (session, scope)
}

/// The ids the window currently holds, once its pages have landed.
fn resident(session: &Session) -> Vec<i64> {
    let _ = session.row_at(0);
    session.settle_for_test();
    (0..session.row_count())
        .filter_map(|row| session.row_at(row))
        .map(|row| row.id)
        .collect()
}

#[test]
fn a_query_returns_hits_and_the_list_windows_over_them() {
    let (session, _) = searchable();
    session.search("quarterly");

    assert_eq!(session.row_count(), 2, "two messages say quarterly");
    assert_eq!(
        resident(&session).len(),
        2,
        "the rows never arrived, so the window is showing a count and nothing else"
    );
    assert!(session.is_searching());
    session.shutdown();
}

#[test]
fn the_rows_come_back_in_rank_order_rather_than_by_date() {
    let (session, _) = searchable();
    session.search("quarterly");
    // The whole reason `message_rows` is used rather than a paged scope: a
    // ranked list re-sorted by date puts the best match wherever its date
    // happens to fall, which is the one thing a ranking must not do. The
    // subject match outranks the body-only one.
    let found = resident(&session);
    assert_eq!(found.len(), 2);
    let first = session.row_at(0).expect("the page landed");
    assert_eq!(
        first.subject.as_deref(),
        Some("Quarterly figures"),
        "the ranking did not survive the page read"
    );
    session.shutdown();
}

#[test]
fn a_query_matching_nothing_is_an_empty_list_rather_than_the_folder() {
    let (session, _) = searchable();
    session.search("zzzqqq");
    assert_eq!(session.row_count(), 0);
    assert!(
        session.is_searching(),
        "an empty result set is still a result set: falling back to the \
         folder would look like the query did nothing"
    );
    session.shutdown();
}

#[test]
fn operators_parse_the_way_the_shared_language_says() {
    let (session, _) = searchable();
    // Not a second parser. `postio-search` reads this on both platforms, and
    // the assertion that matters is that an *operator* reaches it rather than
    // being taken as a literal word -- `subject:quarterly` finding one row
    // and not two is what proves it was understood.
    session.search("subject:quarterly");
    assert_eq!(
        session.row_count(),
        1,
        "`subject:` was searched for as a word instead of read as an operator"
    );
    session.shutdown();
}

#[test]
fn clearing_restores_the_scope_that_was_open() {
    let (session, _) = searchable();
    let before = session.row_count();
    session.search("quarterly");
    assert_ne!(session.row_count(), before, "the search changed nothing");

    session.clear_search();
    assert!(!session.is_searching());
    assert_eq!(
        session.row_count(),
        before,
        "clearing did not come back to the folder that was open"
    );
    session.shutdown();
}

#[test]
fn a_second_query_still_comes_back_to_the_folder() {
    let (session, _) = searchable();
    let before = session.row_count();
    session.search("quarterly");
    // Typing again inside a search must not make the *first search* the thing
    // to return to, or `Escape` would walk backwards through every query.
    session.search("lunch");
    session.clear_search();
    assert_eq!(session.row_count(), before);
    session.shutdown();
}

#[test]
fn opening_a_folder_leaves_the_search_behind() {
    let (session, scope) = searchable();
    session.search("quarterly");
    session.open_scope(scope);
    assert!(!session.is_searching());
    // ...and clearing afterwards is a no-op rather than a jump back into a
    // search nobody is in.
    let showing = session.row_count();
    session.clear_search();
    assert_eq!(session.row_count(), showing);
    session.shutdown();
}

#[test]
fn each_hit_carries_an_excerpt_with_the_match_located() {
    let (session, _) = searchable();
    session.search("quarterly");
    let first = resident(&session).first().copied().expect("a hit");

    let snippet = session.snippet_for(first).expect("a hit has an excerpt");
    assert!(!snippet.text.is_empty(), "an excerpt with no text in it");
    assert!(
        !snippet.ranges.is_empty(),
        "nothing was marked, so the frontend has nothing to highlight"
    );
    // Ranges, not markup: the text must be the plain string, and the marks
    // must point into it. A frontend given pre-marked text would be escaping
    // the other frontend's markup.
    assert!(
        !snippet.text.contains('<'),
        "the excerpt arrived marked up, which is a frontend's decision"
    );
    for range in &snippet.ranges {
        assert!(
            (range.end as usize) <= snippet.text.len(),
            "a match range points past the end of its own excerpt"
        );
        assert!(range.start < range.end, "an empty match range");
    }
    session.shutdown();
}

#[test]
fn a_row_that_is_not_a_hit_has_no_excerpt() {
    let (session, _) = searchable();
    assert_eq!(
        session.snippet_for(1),
        None,
        "an excerpt outside a search is a claim about a query nobody ran"
    );
    session.search("quarterly");
    assert_eq!(session.snippet_for(999_999), None);
    session.shutdown();
}

#[test]
fn searching_drops_what_was_marked() {
    let (session, _) = searchable();
    session.invoke("next_message");
    let _ = session.row_at(0);
    session.settle_for_test();
    session.invoke("toggle_selection");
    assert!(!session.selected_messages().unwrap_or_default().is_empty());

    session.search("quarterly");
    assert_eq!(
        session.selected_messages(),
        Some(Vec::new()),
        "a selection survived into a list of different rows"
    );
    session.shutdown();
}
