use postio_model::{BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn store() -> (test_support::TempStore, postio_model::ids::AccountId) {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

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
        messages.create(&mut message).await.expect("create");
        messages
            .set_body(
                message.id,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_string()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("store the body");
        postio_index::index::index_body(&connection, message.id.get(), Some(body))
            .await
            .expect("index the body");
    }
    drop(connection);
    (database, account.id)
}

async fn run(
    database: &test_support::TempStore,
    account: postio_model::ids::AccountId,
    text: &str,
) -> postio_search::SearchResults {
    let connection = database.connect().await.expect("checkout");
    let query = postio_search::parse(text, chrono::Utc::now().date_naive());
    postio_session::search::execute(
        &connection,
        postio_model::AccountScope::Account(account),
        &query,
        Scope::AllMail,
        postio_search::ResultOrder::Relevance,
    )
    .await
    .expect("the search runs")
}

#[tokio::test]
async fn a_term_finds_the_messages_that_carry_it() {
    let (database, account) = store().await;
    let results = run(&database, account, "quarterly").await;
    assert_eq!(results.hits.len(), 2, "two messages say quarterly");
}

#[tokio::test]
async fn every_hit_carries_an_excerpt_cut_from_its_own_body() {
    let (database, account) = store().await;
    let results = run(&database, account, "quarterly").await;
    for hit in &results.hits {
        assert!(
            !hit.snippet.is_empty(),
            "a text query leaves no hit without an excerpt"
        );
    }
}

#[tokio::test]
async fn a_structured_only_query_leaves_the_excerpts_empty() {
    let (database, account) = store().await;
    let results = run(&database, account, "is:unread").await;
    assert!(!results.hits.is_empty(), "everything here is unread");
    assert!(
        results.hits.iter().all(|hit| hit.snippet.is_empty()),
        "nothing was typed to highlight"
    );
}

#[tokio::test]
async fn a_query_matching_nothing_is_an_empty_result_rather_than_a_failure() {
    let (database, account) = store().await;
    let results = run(&database, account, "aardvark").await;
    assert!(results.hits.is_empty());
}

#[tokio::test]
async fn a_misspelled_word_shows_the_mail_for_the_word_it_meant() {
    let (database, account) = store().await;
    let results = run(&database, account, "quartrly").await;
    assert_eq!(results.hits.len(), 2, "the two messages that say quarterly");
    assert_eq!(
        results.instead,
        Some(postio_search::Instead {
            typed: "quartrly".to_owned(),
            term: "quarterly".to_owned(),
        }),
        "and the surface is told these are for a different word"
    );
    assert!(
        results.hits.iter().all(|hit| !hit.snippet.is_empty()),
        "the excerpt points at the word that matched, not the one typed"
    );
}

#[tokio::test]
async fn an_unfinished_word_shows_the_mail_for_the_word_it_begins() {
    let (database, account) = store().await;
    let results = run(&database, account, "quart").await;
    assert_eq!(results.hits.len(), 2);
    assert_eq!(
        results.instead.map(|instead| instead.term).as_deref(),
        Some("quarterly")
    );
}

#[tokio::test]
async fn a_quoted_word_is_searched_for_exactly() {
    let (database, account) = store().await;
    let results = run(&database, account, "\"quartrly\"").await;
    assert!(results.hits.is_empty());
    assert_eq!(results.instead, None);
}

#[tokio::test]
async fn a_word_that_found_mail_is_shown_as_typed() {
    let (database, account) = store().await;
    let results = run(&database, account, "quarterly").await;
    assert_eq!(results.instead, None);
}

#[tokio::test]
async fn a_search_is_timed_on_the_app_clock() {
    let (database, account) = store().await;
    postio_ui::clock::freeze(chrono::Local::now());
    let results = run(&database, account, "quarterly").await;
    postio_ui::clock::thaw();
    assert_eq!(results.total_hits, 2);
    assert_eq!(results.elapsed, std::time::Duration::ZERO);
}
