//! Opening the store makes its mail findable.
//!
//! `postio-x4e`, the ninth instance of `postio-bl2`: `postio_index`'s schema,
//! its triggers and its executor were all implemented and tested, and nothing
//! in the application created the index. `search_documents` and `messages_fts`
//! did not exist on any real store, so search had nothing to search — even
//! after `postio-svx` made the executor reachable at all.
//!
//! The assertion is the far end again: seed a store the way a synced account
//! looks, open it the way `run` opens it, and search it. Not "the executor
//! works" — `postio-index` proves that itself — but "a store this application
//! opened can be searched", which is the sentence that was false.

use chrono::Utc;
use postio_index::{SearchRequest, search};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_session::ensure_search_index;
use postio_storage::seed::seed_small;
use postio_storage::test_support;

pub fn a_store_the_application_opened_can_be_searched() {
    // Seeded first and indexed after, which is the order every existing
    // account is in: the mail was there long before the index was.
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    assert!(report.message_count > 0, "seeded nothing to find");

    ensure_search_index(&database).expect("the index is part of opening the store");

    let connection = database.connection().expect("a connection");
    // A word every fixture in the corpus has a sender for. Searching for the
    // *sender* rather than a subject also proves the recipients half of the
    // backfill ran, not only the subject column.
    let query = parse("example.com", Utc::now().date_naive());
    let hits = search(
        &connection,
        &SearchRequest {
            account_id: report.account.id,
            query: &query,
            scope: Scope::AllMail,
            limit: 50,
        },
        Utc::now(),
    )
    .expect("the search runs")
    .hits;

    assert!(
        !hits.is_empty(),
        "the store holds {} messages and searching it finds none. Either the \
         index was never created — which is what postio-x4e was — or it was \
         created empty, which is the same bug with the backfill missing.",
        report.message_count
    );
}
