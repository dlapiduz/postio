//! The desktop search, answered for any frontend.
//!
//! Moved from the classic app's search surface (`docs/archive/specs/005-tui-frontend` T018).
//! The desktop ran its two reads -- the hits, then the columns -- on a warm
//! reader of its own; the store's owner runs the same two, with the same
//! executor (`postio_session::search`, `postio_index::executor::facets`),
//! and a frontend asks for each once.
//!
//! Logs here carry counts and outcomes only: never the query, which is the
//! user's words, or anything it matched.

use chrono::NaiveDate;
use postio_model::{AccountScope, MessageId};
use postio_search::facets::{Facets, Scope};
use postio_search::relax::Relaxation;
use postio_search::results::{ConversationOrder, ConversationResults, Match, Source};
use postio_search::{ParsedQuery, ResultOrder, SearchResults};
use postio_session::search::{HIT_LIMIT, execute_with_snippets};
use postio_storage::Store;
use postio_storage::searches::SearchRepository;

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

/// One page of Focus's conversation search, on one reader turn. `None` when
/// the store could not be read or the search did not run.
pub async fn conversations(
    database: &Store,
    account: AccountScope,
    query: &ParsedQuery,
    order: ConversationOrder,
    offset: u32,
    limit: u32,
) -> Option<ConversationResults> {
    let reader = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "no connection to read the index with"))
        .ok()?;
    postio_session::search::conversations(&reader, account, query, order, offset, limit).await
}

/// The passages of the hits on screen, on one reader turn: one body read
/// each, never fetched. Empty when the store could not be read.
pub async fn passages(
    database: &Store,
    query: &ParsedQuery,
    hits: &[(MessageId, Vec<Source>)],
    first_line: postio_search::passage::FirstLine,
) -> Vec<(MessageId, Vec<Match>)> {
    match database.read().await {
        Ok(reader) => postio_session::search::passages(&reader, query, hits, first_line).await,
        Err(error) => {
            tracing::warn!(%error, "no connection to read the passages with");
            Vec::new()
        }
    }
}

/// Every match in the conversation `key`, for Quick Look, on one reader
/// turn. Empty when the store could not be read.
pub async fn conversation_matches(
    database: &Store,
    query: &ParsedQuery,
    key: postio_search::results::ConversationKey,
) -> Vec<postio_search::results::ConversationMatch> {
    match database.read().await {
        Ok(reader) => postio_session::search::conversation_matches(&reader, query, key).await,
        Err(error) => {
            tracing::warn!(%error, "no connection to read a conversation's matches with");
            Vec::new()
        }
    }
}

/// The Files tab's cards, on one reader turn. Empty when the store could
/// not be read.
pub async fn files(
    database: &Store,
    account: AccountScope,
    query: &ParsedQuery,
    offset: u32,
    limit: u32,
) -> Vec<postio_search::results::FileHit> {
    match database.read().await {
        Ok(reader) => postio_session::search::files(&reader, account, query, offset, limit)
            .await
            .unwrap_or_default(),
        Err(error) => {
            tracing::warn!(%error, "no connection to read the files with");
            Vec::new()
        }
    }
}

/// The ways out of a search that found nothing, counted on one reader turn.
/// Empty when none would find anything, or when they could not be counted:
/// an offer that cannot be made is not made.
pub async fn relaxations(
    database: &Store,
    account: AccountScope,
    query: &ParsedQuery,
    today: NaiveDate,
) -> Vec<(Relaxation, u64)> {
    let reader = match database.read().await {
        Ok(reader) => reader,
        Err(error) => {
            tracing::warn!(%error, "no connection to count the ways out with");
            return Vec::new();
        }
    };
    postio_session::search::relaxations(&reader, account, query, today)
        .await
        .unwrap_or_default()
}

/// What a prefix could become, on one reader turn. Empty when the store
/// could not be read: no offer is better than a wrong one.
pub async fn suggest(
    database: &Store,
    account: AccountScope,
    prefix: &str,
    field: Option<postio_search::query::Field>,
) -> postio_search::suggest::Suggestions {
    match database.read().await {
        Ok(reader) => postio_session::search::suggest(&reader, account, prefix, field)
            .await
            .unwrap_or_default(),
        Err(error) => {
            tracing::warn!(%error, "no connection to complete a prefix with");
            postio_search::suggest::Suggestions::default()
        }
    }
}

fn failed(error: postio_storage::Error) -> postio_model::listing::StoreError {
    tracing::warn!(%error, "could not read or write the remembered searches");
    postio_model::listing::StoreError::new(error.to_string())
}

/// The searches a person ran, newest first.
pub async fn recent(
    database: &Store,
) -> Result<Vec<postio_client::protocol::RecentSearch>, postio_model::listing::StoreError> {
    let read = async {
        let reader = database.read().await?;
        SearchRepository::new(&reader).recent().await
    };
    Ok(read
        .await
        .map_err(failed)?
        .into_iter()
        .map(|search| postio_client::protocol::RecentSearch {
            query: search.query,
            last_run_at: search.last_run_at,
            hits: search.hits,
        })
        .collect())
}

/// Record that `query` ran now and matched `hits` conversations.
pub async fn remember(
    database: &Store,
    query: &str,
    hits: u64,
) -> Result<(), postio_model::listing::StoreError> {
    let write = async {
        let (connection, _permit) = database.interactive_write().await?;
        SearchRepository::new(&connection)
            .remember(query, hits, chrono::Utc::now())
            .await
    };
    write.await.map_err(failed)
}

/// Forget one recent search.
pub async fn forget(
    database: &Store,
    query: &str,
) -> Result<(), postio_model::listing::StoreError> {
    let write = async {
        let (connection, _permit) = database.interactive_write().await?;
        SearchRepository::new(&connection).forget(query).await
    };
    write.await.map(|_| ()).map_err(failed)
}

/// The saved search `key` has been viewed: its badge counts from now
/// (D15).
pub async fn mark_seen(
    database: &Store,
    key: &str,
) -> Result<(), postio_model::listing::StoreError> {
    let write = async {
        let (connection, _permit) = database.interactive_write().await?;
        SearchRepository::new(&connection)
            .mark_seen(key, postio_ui::clock::now().to_utc())
            .await
    };
    write.await.map_err(failed)
}

/// The most a badge counts: past it, "99 new" is as much as a quiet badge
/// needs to say.
pub const NEW_CAP: u32 = 99;

/// `(key, total, new)` for each saved search, in the order given. `total`
/// is the conversations it matches, capped as a search's count is. `new`
/// is how many of them have a match received after the search was last
/// viewed ([`mark_seen`]), up to [`NEW_CAP`]; zero for a search never
/// viewed, which is one nobody asked to be told about. One conversation
/// search each, newest first, read only as far as the badge needs. A
/// search that cannot be counted is zero rather than missing, so the row
/// stays.
///
/// Also drops the seen-progress of any saved search not listed: one deleted
/// from `config.toml` leaves an orphan, and this is the read that sees the
/// whole list.
pub async fn saved_counts(
    database: &Store,
    account: AccountScope,
    today: NaiveDate,
    searches: &[(String, String)],
) -> Vec<(String, u64, u64)> {
    let keys: Vec<&str> = searches.iter().map(|(key, _)| key.as_str()).collect();
    let sweep = async {
        let (connection, _permit) = database.interactive_write().await?;
        SearchRepository::new(&connection)
            .forget_seen_except(&keys)
            .await
    };
    if let Err(error) = sweep.await {
        tracing::warn!(%error, "could not drop a deleted saved search's progress");
    }
    let seen = async {
        let reader = database.read().await?;
        let progress = SearchRepository::new(&reader);
        let mut seen = Vec::with_capacity(keys.len());
        for key in &keys {
            seen.push(progress.seen_up_to(key).await?);
        }
        Ok::<_, postio_storage::Error>(seen)
    };
    let seen = seen.await.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read how far the saved searches were seen");
        vec![None; keys.len()]
    });
    let mut counted = Vec::with_capacity(searches.len());
    for ((key, text), seen) in searches.iter().zip(seen) {
        let query = postio_search::parse(text, today);
        let limit = if seen.is_some() { NEW_CAP } else { 1 };
        let found = conversations(
            database,
            account,
            &query,
            ConversationOrder::Newest,
            0,
            limit,
        )
        .await;
        let total = found.as_ref().map_or(0, |found| found.total);
        let new = match (seen, &found) {
            (Some(seen), Some(found)) => found
                .hits
                .iter()
                .take_while(|hit| hit.newest_match > seen)
                .count() as u64,
            _ => 0,
        };
        counted.push((key.clone(), total, new));
    }
    counted
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
