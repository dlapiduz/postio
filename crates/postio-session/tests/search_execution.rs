//! One search path, shared by both frontends (#660).
//!
//! The executor in `postio-index` was always toolkit-free; what was not was
//! the thin layer above it that decides the hit limit and cuts each excerpt.
//! That lived in `postio-app`, where only the GTK build could reach it, so a
//! macOS search would have needed a second copy — and a second copy of "how
//! many hits, and which text gets highlighted" is two products that answer the
//! same query differently the first time either is edited.

use postio_model::{BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A store with three messages, two of which say "quarterly".
fn store() -> (test_support::TempDatabase, postio_model::ids::AccountId) {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let messages = MessageRepository::new(&connection);
    for (offset, subject, body) in [
        (
            0i64,
            "Quarterly review",
            "The quarterly numbers are attached.",
        ),
        (1, "Lunch", "Sandwiches at one."),
        (2, "Quarterly planning", "Planning for the quarterly cycle."),
    ] {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(offset),
        );
        message.subject = Some(subject.to_string());
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).expect("create");
        // Both halves, because they are genuinely different things: the index
        // is what `search` matches against, the stored body is what the
        // excerpt is cut from. A message with one and not the other is a real
        // state -- it gets a hit and no excerpt -- so the fixture has to be
        // explicit about wanting both.
        messages
            .set_body(
                message.id,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_string()),
                    html: None,
                    headers: None,
                },
                BodyState::Full,
            )
            .expect("store the body");
        postio_index::index::index_body(&connection, message.id.get(), Some(body))
            .expect("index the body");
    }
    drop(connection);
    (database, account.id)
}

fn run(
    database: &test_support::TempDatabase,
    account: postio_model::ids::AccountId,
    text: &str,
) -> postio_search::SearchResults {
    let connection = database.connection().expect("checkout");
    let query = postio_search::parse(text, chrono::Utc::now().date_naive());
    postio_session::search::execute(
        &connection,
        account,
        &query,
        Scope::AllMail,
        postio_search::ResultOrder::Relevance,
    )
    .expect("the search runs")
}

#[test]
fn a_term_finds_the_messages_that_carry_it() {
    let (database, account) = store();
    let results = run(&database, account, "quarterly");
    assert_eq!(results.hits.len(), 2, "two messages say quarterly");
}

#[test]
fn every_hit_carries_an_excerpt_cut_from_its_own_body() {
    // The reason this layer exists at all: `snippet()` was an FTS5 function
    // over indexed content, and the body index stores none (#407/#408), so the
    // excerpt has to be reconstructed from the blob. A frontend cutting its
    // own would highlight a different string than the one that matched.
    let (database, account) = store();
    let results = run(&database, account, "quarterly");
    for hit in &results.hits {
        assert!(
            !hit.snippet.is_empty(),
            "a text query leaves no hit without an excerpt"
        );
    }
}

#[test]
fn a_structured_only_query_leaves_the_excerpts_empty() {
    // `is:unread` has nothing to point at. Empty rather than a whole-body
    // excerpt, which is what SQLite did before the reconstruction moved here.
    let (database, account) = store();
    let results = run(&database, account, "is:unread");
    assert!(!results.hits.is_empty(), "everything here is unread");
    assert!(
        results.hits.iter().all(|hit| hit.snippet.is_empty()),
        "nothing was typed to highlight"
    );
}

#[test]
fn a_query_matching_nothing_is_an_empty_result_rather_than_a_failure() {
    let (database, account) = store();
    let results = run(&database, account, "aardvark");
    assert!(results.hits.is_empty());
}
