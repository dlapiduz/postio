use chrono::Utc;
use postio_model::{BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, install};

const COMMON: &str = "quarterly";

async fn indexed(count: usize) -> (postio_storage::Store, postio_model::ids::AccountId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the index schema");

    let repository = MessageRepository::new(&connection);
    for n in 0..count {
        let body = format!("message {n} about the {COMMON} numbers we discussed");
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.subject = Some(format!("Report {n}"));
        message.sync.body_state = BodyState::Full;
        repository.create(&mut message).await.expect("a message");
        repository
            .set_body(
                message.id,
                &StoredBody {
                    text: Some(body.clone()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("a body");
        postio_index::index::index_body(&connection, message.id.get(), Some(&body))
            .await
            .expect("an indexed body");
    }
    let id = account.id;
    drop(connection);
    (database, id)
}

async fn cost_of_searching(
    store: &(postio_storage::Store, postio_model::ids::AccountId),
) -> (usize, usize) {
    let (database, account) = store;
    let connection = database.connect().await.expect("a connection");
    let query = postio_search::parse(COMMON, Utc::now().date_naive());

    let run = async |connection: &postio_storage::Checkout| {
        postio_session::search::execute(
            connection,
            postio_model::AccountScope::Account(*account),
            &query,
            Scope::AllMail,
            postio_search::ResultOrder::Relevance,
        )
        .await
        .expect("the search runs")
    };

    let warm = run(&connection).await;
    let hits = warm.hits.len();

    install(&connection);
    let counts = counted_async(async || {
        let _ = run(&connection).await;
    });
    (counts.await.statements, hits)
}

#[tokio::test]
async fn a_search_costs_the_same_statements_however_large_the_store() {
    let smaller = indexed(120).await;
    let larger = indexed(400).await;

    let (smaller_statements, smaller_hits) = cost_of_searching(&smaller).await;
    let (larger_statements, larger_hits) = cost_of_searching(&larger).await;

    assert!(
        smaller_hits > 50 && larger_hits > 50,
        "both stores have to be past the excerpt cap for this to be flat; \
         got {smaller_hits} and {larger_hits} hits"
    );
    assert!(
        larger_hits > smaller_hits,
        "the larger store returned no more hits ({larger_hits} against \
         {smaller_hits}), so the two runs are not different enough to compare"
    );
    assert_eq!(
        smaller_statements, larger_statements,
        "a search over {larger_hits} hits issued {larger_statements} \
         statements where {smaller_hits} hits issued {smaller_statements}. \
         Past the excerpt cap a search reads nothing further, so this number \
         must not move — one that does is work growing with the store."
    );
}

#[tokio::test]
async fn excerpts_are_cut_for_a_bounded_number_of_hits() {
    let under = indexed(20).await;
    let over = indexed(200).await;

    let (under_statements, under_hits) = cost_of_searching(&under).await;
    let (over_statements, over_hits) = cost_of_searching(&over).await;

    assert!(
        under_hits < 50 && over_hits > 50,
        "this needs one store under the excerpt cap and one over it; got \
         {under_hits} and {over_hits}"
    );
    let growth = over_statements.saturating_sub(under_statements);
    let expected = 50 - under_hits;
    assert_eq!(
        growth, expected,
        "{over_hits} hits cost {growth} more statements than {under_hits} did, \
         where the {expected} excerpts between {under_hits} and the cap of 50 \
         account for exactly {expected}. Past the cap no further excerpt is \
         cut, so anything else is a body read that should not be happening."
    );
}

#[tokio::test]
async fn the_finder_s_search_cuts_one_excerpt_before_it_answers() {
    let store = indexed(120).await;
    let (database, account) = &store;
    let connection = database.connect().await.expect("a connection");
    let query = postio_search::parse(COMMON, Utc::now().date_naive());
    let run = async |snippets: usize| {
        postio_session::search::execute_with_snippets(
            &connection,
            postio_model::AccountScope::Account(*account),
            &query,
            Scope::AllMail,
            postio_search::ResultOrder::Relevance,
            snippets,
        )
        .await
        .expect("the search runs")
    };
    let _warm = run(50).await;
    install(&connection);
    let every = counted_async(async || {
        let _ = run(50).await;
    })
    .await;
    let mut results = None;
    let one = counted_async(async || {
        results = Some(run(1).await);
    })
    .await;
    let results = results.expect("ran");
    assert!(
        !results.hits[0].snippet.is_empty(),
        "the best match has its excerpt"
    );
    assert!(
        results.hits[1..].iter().all(|hit| hit.snippet.is_empty()),
        "no other hit was cut before the answer"
    );
    assert!(
        one.statements + 49 <= every.statements,
        "one excerpt cost {} statements against {} for fifty",
        one.statements,
        every.statements
    );
}
