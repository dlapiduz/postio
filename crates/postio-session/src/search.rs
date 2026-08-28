//! Running a search, and cutting each hit's excerpt.
//!
//! `postio_index::search` is the executor and was always reachable from
//! anywhere; this is the thin layer above it that decides how many hits one
//! run brings back and reconstructs the excerpt each hit shows. That layer
//! lived in `postio-app` until #660, where only the GTK build could reach it.
//!
//! It is here rather than there because it is a *product* decision — how many
//! results, and which text gets highlighted — and two copies of a product
//! decision are two products. `docs/PRODUCT.md` §1 puts finding things among
//! the three jobs Postio must beat the alternatives at; a macOS build with its
//! own hit limit and its own excerpt rule would be answering the same query
//! differently the first time either copy was edited.

use chrono::Utc;
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_search::facets::Scope;
use postio_search::{ParsedQuery, ResultOrder, SearchResults};
use postio_storage::PooledConnection;

/// How many hits one run brings back.
///
/// Not how many *matched* — that is [`SearchResults::total_hits`], which the
/// readout draws and which counts far past this. This is the page the result
/// list is drawn from, and nobody scrolls two hundred results looking for the
/// one they meant.
pub const HIT_LIMIT: u32 = 200;

/// How many hits get an excerpt cut for them.
///
/// Several screens' worth, and short of [`HIT_LIMIT`] on purpose. Each one
/// costs a blob read, and a person who scrolls past fifty results without
/// refining the query is doing something a snippet was not going to help
/// with. Past this the row falls back to the message's own preview, which it
/// already has.
const SNIPPET_HITS: usize = 50;

/// One search against the index, with an excerpt cut for each hit.
///
/// `None` when the query could not run at all — a corrupt index, a locked
/// database. A query that simply matches nothing is `Some` with no hits,
/// because those are different answers and the surface says different things
/// about them.
pub fn execute(
    connection: &PooledConnection,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
) -> Option<SearchResults> {
    let mut results = search(
        connection,
        &SearchRequest {
            // The caller's own scope, passed through. It was hardcoded to
            // `Account` while the composition root opened exactly one account
            // and there was nothing to observe the difference with; #185 gave
            // the sidebar a section per account and a Unified row, and the
            // hardcode outlived its reason by long enough that the executor's
            // unified path had a benchmark and no caller (#961). Moving this
            // function must not put it back.
            account,
            query,
            scope,
            limit: HIT_LIMIT,
            order,
        },
        Utc::now(),
    )
    .map_err(|error| tracing::warn!(%error, "the search did not run"))
    .ok()?;
    snippet_hits(connection, query, &mut results);
    Some(results)
}

/// Cuts each hit's excerpt out of its own body text.
///
/// # Why this is here and not in the executor
///
/// The boundary #408 settled. `snippet()` was an FTS5 function over indexed
/// content and the body index has none — `message_bodies_fts` is
/// `content = ''`, which is the point of it (#407). Reconstructing an excerpt
/// needs the body, the body is in the blob store, and `postio-index` is a
/// rusqlite-only leaf that `check-crate-boundaries.py` keeps that way.
///
/// # Why it agrees with what matched
///
/// `postio_index::index::indexable_text` is the function the *indexer* uses to
/// decide what a message's searchable text is — `text/plain` when there is
/// one, the HTML rendered to text otherwise. Calling the same function means
/// the string highlighted is the string that was indexed, rather than a second
/// guess at it, and `postio_search::highlight`'s token rule is FTS5's own. A
/// message with no local body gets no excerpt rather than a wrong one.
fn snippet_hits(connection: &PooledConnection, query: &ParsedQuery, results: &mut SearchResults) {
    let terms = postio_search::highlight::terms(query);
    if terms.is_empty() {
        // A structured-only query — `is:unread`, `in:archive` — has nothing to
        // point at, and every hit's snippet stays empty exactly as it did when
        // SQLite was cutting them.
        return;
    }
    for hit in results.hits.iter_mut().take(SNIPPET_HITS) {
        let body = crate::reading::load_body(connection, hit.message_id);
        if let Some(text) = postio_index::index::indexable_text(&body) {
            hit.snippet = postio_search::highlight::snippet(&text, &terms);
        }
    }
}
