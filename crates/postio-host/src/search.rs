//! The desktop search, answered for any frontend.
//!
//! Moved from `postio-app`'s search surface (`specs/005-tui-frontend` T018).
//! The desktop ran its two reads -- the hits, then the columns -- on a warm
//! reader of its own; the store's owner runs the same two, with the same
//! executor (`postio_session::search`, `postio_index::executor::facets`),
//! and a frontend asks for each once.
//!
//! Logs here carry counts and outcomes only: never the query, which is the
//! user's words, or anything it matched.

use postio_model::AccountScope;
use postio_search::facets::{Facets, Scope};
use postio_search::{ParsedQuery, ResultOrder, SearchResults};
use postio_session::search::{HIT_LIMIT, execute_with_snippets};
use postio_storage::Store;

/// The hits for `query`, excerpted for the first `snippets`, on one reader
/// turn. `None` when the store could not be read or the search did not run.
///
/// A turn on a warm reader rather than a connection of its own: a search run
/// took four cold caches per keystroke that way (#1602).
pub async fn hits(
    database: &Store,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    order: ResultOrder,
    snippets: usize,
) -> Option<SearchResults> {
    let reader = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "no connection to read the index with"))
        .ok()?;
    execute_with_snippets(&reader, account, query, scope, order, snippets).await
}

/// What the results' columns say about `query`: every scope's count and the
/// refinements measured against it. `None` when they did not run.
///
/// Counted in relevance order and to the hit limit whatever the list is
/// sorted by: the columns describe the result set, not its order.
pub async fn facets(
    database: &Store,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
) -> Option<Facets> {
    let reader = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "no connection to read the index with"))
        .ok()?;
    postio_index::executor::facets(
        &reader,
        &postio_index::SearchRequest {
            account,
            query,
            scope,
            limit: HIT_LIMIT,
            order: ResultOrder::Relevance,
        },
    )
    .await
    .map_err(|error| tracing::warn!(%error, "the facet counts did not run"))
    .ok()
}

/// A message's stored words, for a search preview; empty when none are
/// here. Never fetched.
pub async fn stored_body(
    database: &Store,
    message: postio_model::MessageId,
) -> postio_model::MessageBody {
    match database.read().await {
        Ok(reader) => postio_session::reading::load_body(&reader, message).await,
        Err(error) => {
            tracing::warn!(%error, "no connection to read a body with");
            postio_model::MessageBody::default()
        }
    }
}
