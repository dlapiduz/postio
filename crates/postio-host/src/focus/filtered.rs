//! Focus's Filtered view (spec 007 US9, screen 21): what filing archived,
//! each message with its reason, newest first, and the tabs' counts.
//!
//! A page is two reads: the standing decisions, through their index, and
//! the list rows of the messages they name, in one batch. The tabs are one
//! statement. Nothing walks a table.

use postio_client::protocol::FilteredRow;
use postio_model::listing::StoreError;
use postio_storage::repository::{FilterDecisionRepository, FilterReason};

use crate::Inner;

/// Each reason's name, as the store spells it, with how many messages it
/// keeps filtered now, in the tabs' order.
pub(crate) async fn tabs(inner: &Inner) -> Result<Vec<(String, u32)>, StoreError> {
    let reader = inner.wiring.database.read().await?;
    Ok(FilterDecisionRepository::new(&reader)
        .tabs()
        .await?
        .into_iter()
        .map(|(reason, count)| (reason.as_str().to_owned(), count))
        .collect())
}

/// `limit` filtered messages from `offset`, newest first, of the reason
/// named `reason` or of every reason.
pub(crate) async fn page(
    inner: &Inner,
    reason: Option<String>,
    offset: u32,
    limit: u32,
) -> Result<Vec<FilteredRow>, StoreError> {
    let reason = match reason.as_deref() {
        Some(name) => Some(
            FilterReason::from_name(name)
                .ok_or_else(|| StoreError::new(format!("No filter reason is called {name}")))?,
        ),
        None => None,
    };
    let decisions = {
        let reader = inner.wiring.database.read().await?;
        FilterDecisionRepository::new(&reader)
            .filtered(reason, offset, limit)
            .await?
    };
    let ids = decisions.iter().map(|decision| decision.message).collect();
    let rows = inner.wiring.store.message_rows(ids).await?;
    Ok(decisions
        .into_iter()
        .filter_map(|decision| {
            let message = rows.iter().find(|row| row.id == decision.message)?.clone();
            Some(FilteredRow {
                message,
                reason: decision.reason.as_str().to_owned(),
                source: decision.source,
                at: decision.decided_at,
            })
        })
        .collect())
}
