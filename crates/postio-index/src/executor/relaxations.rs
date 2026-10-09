//! How many conversations each way out of an empty search would find
//! (spec 010, US6, research R7).
//!
//! [`postio_search::relax::relax`] proposes the looser searches; this counts
//! them, one statement each, in the unit and over the match the results
//! would show: conversations, every folder but drafts, junk and trash, and
//! the sets a conversation search reads apart ([`super::IdSet`]) read apart
//! here too, in the same statement as the walk they narrow.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use postio_model::AccountScope;
use postio_search::facets::Scope;
use postio_search::relax::Relaxation;
use postio_search::results::TOTAL_HITS_CAP;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

use super::{Form, HITS_JOIN, Plan, SearchRequest};
use crate::error::Result;

/// Each relaxation's conversation total, in the order given: what
/// `search_conversations` would say its `total` is. Past the cap a count is
/// a floor, as every count is.
///
/// Counted to [`TOTAL_HITS_CAP`], not the conversation walk's own, smaller
/// cap (D30): this walk reads a conversation id a row, not the facets'
/// columns, and five of them cost under 4 ms at the larger cap. A count
/// between the two is then more exact than the search it opens, whose
/// total says "5,000+"; the two agree, as a floor and a count do.
pub async fn relaxation_counts(
    connection: &Connection,
    account: AccountScope,
    relaxations: &[Relaxation],
    today: NaiveDate,
) -> Result<Vec<u64>> {
    let mut counts = Vec::with_capacity(relaxations.len());
    for relaxation in relaxations {
        let query = postio_search::parse(&relaxation.query, today);
        counts.push(count(connection, account, &query).await?);
    }
    Ok(counts)
}

/// One query's conversations, counted in one statement: the walk of its
/// match under its conditions, each row with its conversation, then the
/// ids of every set it has, intersected here.
///
/// The walk is driven by the free text when there is any, as every search
/// is, and otherwise by the first set that must hold -- `from:ada` walks
/// Ada's messages, not the mailbox -- so a variant costs what the query it
/// stands for costs.
async fn count(
    connection: &Connection,
    account: AccountScope,
    query: &postio_search::ParsedQuery,
) -> Result<u64> {
    Ok(counts(
        connection,
        account,
        std::slice::from_ref(query),
        TOTAL_HITS_CAP,
    )
    .await?
    .first()
    .map_or(0, |counted| counted.conversations))
}

/// One query's conversations, and whether the walk stopped at its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Counted {
    /// The conversations among the matches walked.
    pub(super) conversations: u64,
    /// The walk stopped at the cap: `conversations` is a floor.
    pub(super) capped: bool,
}

/// Each query's conversations, as [`count`] counts one, in **one**
/// statement whatever their number: every query's arms in one union, each
/// row tagged with the query it is for. What a completion's suggestions
/// are counted with (spec 010 US7), where a statement each would spend the
/// keystroke's budget on round trips.
///
/// The walk of each stops at `cap` matches, and a count that stopped says
/// so: the relaxations pass [`TOTAL_HITS_CAP`], the total a conversation
/// search would show; a completion passes its own, smaller cap (D29).
pub(super) async fn counts(
    connection: &Connection,
    account: AccountScope,
    queries: &[postio_search::ParsedQuery],
    cap: u64,
) -> Result<Vec<Counted>> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    let limit = i64::try_from(cap).unwrap_or(i64::MAX);
    let mut arms: Vec<String> = Vec::new();
    let mut params: Vec<turso::Value> = Vec::new();
    let mut plans = Vec::with_capacity(queries.len());
    for (index, query) in queries.iter().enumerate() {
        let plan = Plan::build_sets(&SearchRequest {
            account,
            query,
            scope: Scope::AllMail,
            limit: 0,
            order: postio_search::ResultOrder::Relevance,
        });
        let conditions = plan.conditions.join(" AND ");
        let driver = match plan.has_match {
            true => None,
            false => plan.sets.iter().position(|set| !set.negated),
        };
        let walk = match driver {
            _ if plan.has_match => {
                params.extend(plan.params_for(Form::Driven));
                format!("{HITS_JOIN} WHERE {}", plan.where_sql(Form::Driven))
            }
            Some(driver) => {
                params.extend(plan.sets[driver].params.iter().cloned());
                params.extend(plan.params.iter().cloned());
                format!(
                    "FROM ({}) d CROSS JOIN messages m ON m.id = d.message_id WHERE {conditions}",
                    plan.sets[driver].sql
                )
            }
            None => {
                params.extend(plan.params.iter().cloned());
                format!("FROM messages m WHERE {conditions}")
            }
        };
        // Rows, not messages: the union's two halves, so twice the cap.
        params.push(turso::Value::Integer(limit.saturating_mul(2)));
        arms.push(format!(
            "SELECT * FROM (SELECT {index} AS q, -1 AS k, m.id AS id,
                                     coalesce(m.thread_id, -m.id) AS conv,
                                     m.content_id AS content
                              {walk} LIMIT ?)"
        ));
        for (key, set) in plan.sets.iter().enumerate() {
            if Some(key) == driver {
                continue;
            }
            arms.push(format!(
                "SELECT {index}, {key}, x.message_id, NULL, NULL FROM ({}) x",
                set.sql
            ));
            params.extend(set.params.iter().cloned());
        }
        plans.push((plan, driver));
    }

    let mut walked: Vec<Vec<(i64, i64, Option<i64>)>> = vec![Vec::new(); queries.len()];
    let mut sets: Vec<Vec<HashSet<i64>>> = plans
        .iter()
        .map(|(plan, _)| vec![HashSet::new(); plan.sets.len()])
        .collect();
    sql::each(connection, &arms.join(" UNION ALL "), params, |row| {
        let query = usize::try_from(row.col::<i64>(0)?).unwrap_or(usize::MAX);
        let key: i64 = row.col(1)?;
        let id: i64 = row.col(2)?;
        match usize::try_from(key) {
            Ok(set) => {
                if let Some(ids) = sets.get_mut(query).and_then(|sets| sets.get_mut(set)) {
                    ids.insert(id);
                }
            }
            Err(_) => {
                if let Some(rows) = walked.get_mut(query) {
                    rows.push((id, row.col(3)?, row.col(4)?));
                }
            }
        }
        Ok(true)
    })
    .await?;

    Ok(plans
        .iter()
        .zip(sets)
        .zip(walked)
        .map(|(((plan, driver), sets), walked)| {
            let holds = |id: i64| {
                plan.sets
                    .iter()
                    .zip(&sets)
                    .enumerate()
                    .all(|(key, (set, ids))| {
                        Some(key) == *driver || ids.contains(&id) != set.negated
                    })
            };
            // One match per content, as `search_conversations` folds it: a
            // message filed in two folders (#1780) is counted once, in the
            // conversation of its earliest occurrence that holds.
            let mut kept: HashMap<Held, (i64, i64)> = HashMap::new();
            // A walk that filled its LIMIT may have more behind it.
            let mut capped = walked.len() as u64 >= cap.saturating_mul(2);
            for (id, conversation, content) in walked {
                if !holds(id) {
                    continue;
                }
                let key = content.map_or(Held::Message(id), Held::Content);
                if let Some(earliest) = kept.get_mut(&key) {
                    if id < earliest.0 {
                        *earliest = (id, conversation);
                    }
                    continue;
                }
                if kept.len() as u64 >= cap {
                    capped = true;
                    break;
                }
                kept.insert(key, (id, conversation));
            }
            let conversations: HashSet<i64> = kept
                .values()
                .map(|(_, conversation)| *conversation)
                .collect();
            Counted {
                conversations: conversations.len() as u64,
                capped,
            }
        })
        .collect())
}

/// What one counted match is: its content, or the message itself when it has
/// none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Held {
    Content(i64),
    Message(i64),
}
