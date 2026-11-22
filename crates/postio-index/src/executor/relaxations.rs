//! How many conversations each way out of an empty search would find, in the unit and over the
//! match the results would show, one statement each.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use postio_model::AccountScope;
use postio_search::facets::Scope;
use postio_search::relax::Relaxation;
use postio_search::results::{Counted, TOTAL_HITS_CAP};
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

use super::{COPY_KEY, Form, HITS_JOIN, Plan, SearchRequest};
use crate::error::Result;

/// Counted to [`TOTAL_HITS_CAP`], not the walk's smaller cap (D30): this walk reads a conversation
/// id a row, and five cost under 4 ms at the larger cap.
/// Past the cap a count is a floor and says so (`capped`).
pub async fn relaxation_counts(
    connection: &Connection,
    account: AccountScope,
    relaxations: &[Relaxation],
    today: NaiveDate,
) -> Result<Vec<Counted>> {
    let mut counts = Vec::with_capacity(relaxations.len());
    for relaxation in relaxations {
        let query = postio_search::parse(&relaxation.query, today);
        counts.push(count(connection, account, &query).await?);
    }
    Ok(counts)
}

/// Driven by the free text when there is any, otherwise by the first set that must hold, so a
/// variant costs what the query it stands for costs.
async fn count(
    connection: &Connection,
    account: AccountScope,
    query: &postio_search::ParsedQuery,
) -> Result<Counted> {
    Ok(counts(
        connection,
        account,
        std::slice::from_ref(query),
        TOTAL_HITS_CAP,
    )
    .await?
    .first()
    .copied()
    .unwrap_or_default())
}

/// One statement whatever their number, each row tagged with its query: a statement each would
/// spend the keystroke budget on round trips.
/// The walk stops at `cap`; relaxations pass [`TOTAL_HITS_CAP`], a completion its own smaller cap
/// (D29).
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
        params.push(turso::Value::Integer(limit.saturating_mul(2)));
        arms.push(format!(
            "SELECT * FROM (SELECT {index} AS q, -1 AS k, m.id AS id,
                                     coalesce(m.thread_id, -m.id) AS conv,
                                     m.content_id AS content, {COPY_KEY} AS copy
                              {walk} LIMIT ?)"
        ));
        for (key, set) in plan.sets.iter().enumerate() {
            if Some(key) == driver {
                continue;
            }
            arms.push(format!(
                "SELECT {index}, {key}, x.message_id, NULL, NULL, NULL FROM ({}) x",
                set.sql
            ));
            params.extend(set.params.iter().cloned());
        }
        plans.push((plan, driver));
    }

    let mut walked: Vec<Vec<Walked>> = vec![Vec::new(); queries.len()];
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
                    rows.push((id, row.col(3)?, row.col(4)?, row.col(5)?));
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
            // One match per content, as `search_conversations` folds it: a message filed in two
            // folders (#1780), or an exact copy with content of its own (#1526), is counted once.
            let mut kept: HashMap<Held, (i64, i64)> = HashMap::new();
            let mut copies: HashMap<String, Held> = HashMap::new();
            let mut aliases: HashMap<Held, Held> = HashMap::new();
            let mut capped = walked.len() as u64 >= cap.saturating_mul(2);
            for (id, conversation, content, copy) in walked {
                if !holds(id) {
                    continue;
                }
                let own = content.map_or(Held::Message(id), Held::Content);
                let key = match aliases.get(&own) {
                    Some(key) => *key,
                    None => {
                        let key = copy
                            .map(|copy| *copies.entry(copy).or_insert(own))
                            .unwrap_or(own);
                        aliases.insert(own, key);
                        key
                    }
                };
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

type Walked = (i64, i64, Option<i64>, Option<String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Held {
    Content(i64),
    Message(i64),
}
