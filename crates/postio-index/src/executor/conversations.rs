//! Conversation search (spec 010): the match answered as conversations, with
//! every facet and the months histogram, in a fixed number of statements.
//!
//! # One walk, folded in Rust
//!
//! [`search`](super::search) reads the match twice or more -- a count, a
//! pool, a hydrate -- and [`facets`](super::facets) walks it again per scope.
//! A conversation search needs the whole match whatever the page: a thread
//! is one row however many of its messages matched, the total is of
//! conversations, and every facet is a count of conversations over the same
//! capped match (D3). So the match is read **once**, as a *narrow
//! projection* (research R2): the capped match joined to `messages`, each row
//! a handful of integers -- its conversation, folder, dates and flags, its
//! two text scores, and three short correlated lookups (its people, its
//! labels, whether it has an open marker). [`Fold`] takes the rows as they
//! stream and builds the conversations; the facets are counted from those.
//!
//! The walk stops as soon as it holds [`TOTAL_HITS_CAP`] messages, so a word
//! in every message of a large store reads the cap's worth of rows and no
//! more.
//!
//! A filter that is a *set* of messages -- `from:`, `to:`, `subject:`,
//! `filename:`, `list:`, `group:`, `label:`, `has:action`, a negated word --
//! is read once, as ids, in one statement with the words' own matches, and
//! the walk is then keyed by the messages that survive ([`Fold::walk_sets`]).
//! Left in SQL as `m.id IN (…)`, this engine tests it per row walked at a
//! cost that grows with the set: `from:` alone took 440 ms on 20,000
//! messages, and takes 6 as a set.
//!
//! Then one hydrate for the rows that will be ranked or shown, and one read
//! of the folders for how much was searched. Three statements, or four with
//! a set, whatever the match size; `index_suite::search_statement_budget`
//! holds that.
//!
//! # Ranking
//!
//! The executor's own: a message's text score is its metadata score plus
//! [`BODY_SCORE_WEIGHT`] of its body score, a conversation's best message is
//! its best-scoring match, and the conversations are ordered by that
//! message the way [`search`](super::search) orders its pool -- first by the
//! pool ordering (text and age, [`POOL_AGE_WEIGHT_PER_YEAR`]), then, for the
//! best [`CONVERSATION_POOL`], by [`rank_score`](super::rank_score) and its
//! bands ([`relevance_keys`]), with ties going to the conversation more of
//! whose messages matched (research R3). The order never depends on which
//! page is asked for, so paging cannot reorder what was already shown.
//!
//! Unlike [`search`](super::search), nothing is ordered in SQL: the fold
//! holds every match already, and sorting ten thousand numbers in Rust is
//! not what costs.

use std::collections::HashMap;
use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use postio_model::{
    AccountScope, AddressId, AttachmentId, EmailAddress, LabelId, MailboxId, MessageId,
};
use postio_search::ParsedQuery;
use postio_search::facets::{Count, MonthCount, Scope, SearchFacets, months_ending, preset_starts};
use postio_search::query::Filter;
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationOrder, ConversationResults, FacetNames, Match,
    RankReason, Source, TOTAL_HITS_CAP,
};
use postio_storage::Connection;
use postio_storage::repository::from_millis;
use postio_storage::sql::{self, RowExt as _};

use super::{
    AGED_FROM, BODY_SCORE_WEIGHT, Candidate, Form, HITS_JOIN_WITH_FILES, MILLIS_PER_YEAR,
    POOL_AGE_WEIGHT_PER_YEAR, Plan, SearchRequest, names_a_folder, relevance_keys, scope_role,
};
use crate::error::Result;

/// How many conversations, best by the pool ordering, are ranked in full.
///
/// A constant rather than a multiple of the page, as `search`'s pool is,
/// because the order must not depend on the page asked for: the dropdown's
/// four and the results' first fifty are the same four.
const CONVERSATION_POOL: usize = 250;

/// Two-way frequency (D21) from which a sender is "frequent": the second of
/// [`known`](super::known)'s four steps, ten letters either way.
const FREQUENT_SENDER: i64 = 10;

/// A conversation search.
#[derive(Debug, Clone, Copy)]
pub struct ConversationRequest<'a> {
    /// Which accounts to search.
    pub account: AccountScope,
    /// The query.
    pub query: &'a ParsedQuery,
    /// Best match or newest first.
    pub order: ConversationOrder,
    /// How many conversations to skip.
    pub offset: u32,
    /// How many to return: four for the dropdown, a page for the results.
    pub limit: u32,
    /// The day the months histogram ends with (D4).
    pub today: NaiveDate,
}

impl ConversationRequest<'_> {
    /// The same match as [`search`](super::search) would make of it: every
    /// folder but drafts, junk and trash, unless the query names one.
    fn as_search(&self) -> SearchRequest<'_> {
        SearchRequest {
            account: self.account,
            query: self.query,
            scope: Scope::AllMail,
            limit: self.limit,
            order: self.order.into(),
        }
    }
}

/// Searches as conversations: one page of hits, the total, and the facets.
///
/// `now` is the ranking's clock, as in [`search`](super::search).
pub async fn search_conversations(
    connection: &Connection,
    request: &ConversationRequest<'_>,
    now: DateTime<Utc>,
) -> Result<ConversationResults> {
    let start = Instant::now();
    let plan = Plan::build_sets(&request.as_search());
    let fold = if plan.sets.is_empty() {
        Fold::walk(connection, &plan).await?
    } else {
        Fold::walk_sets(connection, &plan).await?
    };
    let conversations = fold.conversations();

    let order = order(&fold, &conversations, request.order, now);
    let offset = (request.offset as usize).min(order.len());
    let end = offset
        .saturating_add(request.limit as usize)
        .min(order.len());
    let page = &order.ranked[offset..end];

    // The rows that need more than the fold holds: the ranked pool, and the
    // page wherever it falls.
    let mut wanted: Vec<i64> = order
        .pool
        .iter()
        .map(|at| fold.found[conversations[*at].best].id)
        .collect();
    wanted.extend(page.iter().map(|at| fold.found[conversations[*at].best].id));
    wanted.sort_unstable();
    wanted.dedup();
    let rows = hydrate(connection, request.account, &wanted).await?;
    let order = order.rank(&fold, &conversations, &rows, request.query, now);
    let page: Vec<usize> = order[offset..end].to_vec();

    // Where in which attachment, for the shown rows an attachment's text
    // matched: one statement, and none when no row did.
    let in_files: Vec<MessageId> = page
        .iter()
        .map(|at| &fold.found[conversations[*at].best])
        .filter(|found| found.in_file)
        .map(|found| MessageId::new(found.id))
        .collect();
    let files = if in_files.is_empty() {
        Vec::new()
    } else {
        super::file_matches(connection, request.query, &in_files).await?
    };

    let terms = Terms::of(request.query);
    let hits: Vec<ConversationHit> = page
        .iter()
        .map(|at| hit(&fold, &conversations[*at], &rows, &files, &terms, now))
        .collect();

    let searched = searched(connection, request).await?;
    let elapsed = start.elapsed();
    // Shape and cost, never the query: see `search_as`.
    tracing::debug!(
        terms = request.query.text_terms().count(),
        filters = request.query.filters().count(),
        matched = fold.found.len(),
        conversations = conversations.len(),
        hits = hits.len(),
        capped = fold.capped(),
        elapsed_ms = elapsed.as_millis(),
        "conversation search finished"
    );
    Ok(ConversationResults {
        hits,
        total: conversations.len() as u64,
        capped: fold.capped(),
        messages_searched: searched.messages,
        corpus_complete: searched.bodies,
        contents_complete: searched.contents,
        facets: facets(&conversations, request.today, fold.capped()),
        files: conversations
            .iter()
            .map(|conversation| conversation.files)
            .sum(),
        people: people_in(&conversations),
        // Names are the session's to read, for the counts it keeps.
        names: FacetNames::default(),
        elapsed,
    })
}

/// One message the match holds, as the Files tab reads it.
pub(super) struct Carrier {
    /// The message.
    pub(super) id: MessageId,
    /// Whether an attachment's text matched it.
    pub(super) in_file: bool,
}

/// The messages the request's match holds that carry attachments: the
/// same walk [`search_conversations`] makes, capped as it is, one
/// occurrence per content.
pub(super) async fn carriers(
    connection: &Connection,
    request: &ConversationRequest<'_>,
) -> Result<Vec<Carrier>> {
    let plan = Plan::build_sets(&request.as_search());
    let fold = if plan.sets.is_empty() {
        Fold::walk(connection, &plan).await?
    } else {
        Fold::walk_sets(connection, &plan).await?
    };
    Ok(fold
        .found
        .iter()
        .filter(|found| found.files > 0)
        .map(|found| Carrier {
            id: MessageId::new(found.id),
            in_file: found.in_file,
        })
        .collect())
}

/// One person the match reaches, as the People tab counts them.
pub(super) struct Correspondent {
    /// Their address row.
    pub(super) address: i64,
    /// Matched messages from them.
    pub(super) from: u64,
    /// Matched messages to them (to, cc, bcc) not also from them.
    pub(super) to: u64,
    /// The newest of those, in milliseconds.
    pub(super) last: i64,
}

/// Everyone the request's match is from or to, each with how many of the
/// matched messages and the newest: the same walk [`search_conversations`]
/// makes, capped as it is, one occurrence per content -- the people its
/// People count is of.
pub(super) async fn correspondents(
    connection: &Connection,
    request: &ConversationRequest<'_>,
) -> Result<Vec<Correspondent>> {
    let plan = Plan::build_sets(&request.as_search());
    let fold = if plan.sets.is_empty() {
        Fold::walk(connection, &plan).await?
    } else {
        Fold::walk_sets(connection, &plan).await?
    };
    let mut people: HashMap<i64, Correspondent> = HashMap::new();
    for found in &fold.found {
        let mut met = |address: i64, from: bool| {
            let person = people.entry(address).or_insert(Correspondent {
                address,
                from: 0,
                to: 0,
                last: i64::MIN,
            });
            if from {
                person.from += 1;
            } else {
                person.to += 1;
            }
            person.last = person.last.max(found.received_at);
        };
        let mut senders = found.senders.clone();
        senders.sort_unstable();
        senders.dedup();
        for address in &senders {
            met(*address, true);
        }
        let mut recipients = found.recipients.clone();
        recipients.sort_unstable();
        recipients.dedup();
        for address in recipients.iter().filter(|id| !senders.contains(id)) {
            met(*address, false);
        }
    }
    Ok(people.into_values().collect())
}

/// The words a file's name is checked for: the free text and every
/// `filename:` value; empty when the query names no word for a file.
pub(super) fn file_terms(query: &ParsedQuery) -> Vec<String> {
    Terms::of(query).files
}

// ---------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------

/// One matched message, as the projection gave it.
#[derive(Debug, Clone)]
struct Found {
    id: i64,
    key: ConversationKey,
    mailbox: i64,
    received_at: i64,
    aged_from: i64,
    unread: bool,
    flagged: bool,
    answered: bool,
    attachment: bool,
    action: bool,
    /// Attachments it carries: the Files tab's count.
    files: u64,
    /// Metadata score plus [`BODY_SCORE_WEIGHT`] of the body's, negative
    /// is better, as `bm25` was.
    text: f64,
    /// Whether the body index matched it.
    in_body: bool,
    /// Whether an attachment's text matched it.
    in_file: bool,
    senders: Vec<i64>,
    recipients: Vec<i64>,
    labels: Vec<i64>,
}

/// One message's text scores, from whichever indexes matched it.
#[derive(Debug, Clone, Copy, Default)]
struct Scores {
    /// Metadata, plus [`BODY_SCORE_WEIGHT`] of the body's and the files':
    /// negative is better, as `bm25` was.
    text: f64,
    in_body: bool,
    in_file: bool,
}

/// The match, read once.
#[derive(Debug, Default)]
struct Fold {
    found: Vec<Found>,
    at: HashMap<i64, usize>,
    /// Each content's place in [`Fold::found`]: a message filed in two
    /// folders is one content (#1780), and one match.
    contents: HashMap<i64, usize>,
    /// The occurrences another of the same content stands for, so that
    /// their rows from the other index are passed over too.
    passed: std::collections::HashSet<i64>,
}

impl Fold {
    /// Streams the projection into a fold: the one walk of the match,
    /// stopped once it holds the cap's worth of messages.
    ///
    /// Driven by the free-text match when there is one, as `count` is, and
    /// otherwise a walk of `messages` under the plan's conditions.
    async fn walk(connection: &Connection, plan: &Plan) -> Result<Self> {
        let mut fold = Fold::default();
        let mut params = plan.params_for(Form::Driven);
        // Rows, not messages: the union hands a message back once per index
        // it matched in, so twice the cap bounds the walk however it falls.
        params.push(turso::Value::Integer(cap() * 2));
        let source = if plan.has_match {
            Walk::Matched
        } else {
            Walk::Messages
        };
        sql::each(connection, &projection_sql(plan, source), params, |row| {
            fold.take(row, None)
        })
        .await?;
        Ok(fold)
    }

    /// [`Fold::walk`] for a plan with sets ([`Plan::build_sets`]): the free
    /// text and every set read first, in one statement, and intersected
    /// here; then the projection keyed by the messages that survive.
    ///
    /// Two statements where [`Fold::walk`] is one, and far cheaper whenever
    /// a set is in play: `budget from:ada` reads the word's matches and
    /// Ada's messages as two lists of ids, and walks `messages` only for the
    /// few in both. Tested in SQL, each of the word's matches would have
    /// been compared against every one of Ada's (see [`super::IdSet`]).
    async fn walk_sets(connection: &Connection, plan: &Plan) -> Result<Self> {
        // The free text is read here only when a set must also hold: then
        // the walk is keyed by the messages in both. With refusals alone,
        // the walk is the match itself, and a refused message is skipped as
        // it streams past.
        let keeps = plan.sets.iter().any(|set| !set.negated);
        let read_text = plan.has_match && keeps;
        let mut arms: Vec<String> = Vec::new();
        let mut params: Vec<turso::Value> = Vec::new();
        if read_text {
            // The match's own two arms, with their scores: `fts_score` bare
            // and with the match's own parameter, as `HITS_JOIN` explains.
            // Both indexes are keyed by content: each match is joined to
            // every message carrying it, and the fold keeps one.
            arms.push(format!(
                "SELECT -2, m.id, h.meta, NULL, NULL
                   FROM (SELECT content_id, fts_score({META}, ?1) AS meta
                           FROM search_documents WHERE fts_match({META}, ?1)) h
                   CROSS JOIN messages m ON m.content_id = h.content_id"
            ));
            arms.push(
                "SELECT -1, m.id, NULL, h.body, NULL
                   FROM (SELECT content_id, fts_score(body_search, ?2) AS body
                           FROM message_search_bodies WHERE fts_match(body_search, ?2)) h
                   CROSS JOIN messages m ON m.content_id = h.content_id"
                    .to_owned(),
            );
            // What attachments say, one row per content however many of
            // its units match ([`HITS_JOIN_WITH_FILES`]).
            arms.push(
                "SELECT -3, m.id, NULL, NULL, h.file
                   FROM (SELECT content_id, max(unit_score) AS file
                           FROM (SELECT content_id, fts_score(text_search, ?2) AS unit_score
                                   FROM attachment_passages WHERE fts_match(text_search, ?2))
                          GROUP BY content_id) h
                   CROSS JOIN messages m ON m.content_id = h.content_id"
                    .to_owned(),
            );
            params.extend(plan.match_params(Form::Driven));
        }
        for (key, set) in plan.sets.iter().enumerate() {
            arms.push(format!(
                "SELECT {key}, x.message_id, NULL, NULL, NULL FROM ({}) x",
                set.sql
            ));
            params.extend(set.params.iter().cloned());
        }

        let mut scores: HashMap<i64, Scores> = HashMap::new();
        let mut sets: Vec<std::collections::HashSet<i64>> =
            vec![Default::default(); plan.sets.len()];
        sql::each(connection, &arms.join(" UNION ALL "), params, |row| {
            let key: i64 = row.col(0)?;
            let id: i64 = row.col(1)?;
            match usize::try_from(key) {
                Ok(set) => {
                    sets[set].insert(id);
                }
                // A free-text arm: the score negated here, outside the
                // projection, as `fetch_candidates` negates it.
                Err(_) => {
                    let meta: Option<f64> = row.col(2)?;
                    let body: Option<f64> = row.col(3)?;
                    let file: Option<f64> = row.col(4)?;
                    let entry = scores.entry(id).or_default();
                    entry.text -= meta.unwrap_or(0.0)
                        + BODY_SCORE_WEIGHT * (body.unwrap_or(0.0) + file.unwrap_or(0.0));
                    entry.in_body |= body.is_some();
                    entry.in_file |= file.is_some();
                }
            }
            Ok(true)
        })
        .await?;

        let (keep, refuse): (Vec<_>, Vec<_>) = plan
            .sets
            .iter()
            .zip(sets)
            .partition(|(set, _)| !set.negated);
        let keep: Vec<std::collections::HashSet<i64>> =
            keep.into_iter().map(|(_, ids)| ids).collect();
        let refuse: std::collections::HashSet<i64> =
            refuse.into_iter().flat_map(|(_, ids)| ids).collect();
        let wanted = |id: &i64| keep.iter().all(|set| set.contains(id)) && !refuse.contains(id);
        // The messages to walk: the free text's matches, else the smallest
        // set that must hold; with neither, every message, less the refused.
        let candidates: Option<Vec<i64>> = if read_text {
            Some(scores.keys().copied().filter(wanted).collect())
        } else {
            keep.iter()
                .min_by_key(|set| set.len())
                .map(|smallest| smallest.iter().copied().filter(wanted).collect())
        };

        let mut fold = Fold::default();
        match candidates {
            Some(mut ids) => {
                if ids.is_empty() {
                    return Ok(fold);
                }
                ids.sort_unstable();
                let mut params = vec![turso::Value::Text(format!(
                    "[{}]",
                    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
                ))];
                params.extend(plan.params.iter().cloned());
                params.push(turso::Value::Integer(cap()));
                sql::each(
                    connection,
                    &projection_sql(plan, Walk::Keyed),
                    params,
                    |row| {
                        let id: i64 = row.col(0)?;
                        fold.take(row, scores.get(&id).copied())
                    },
                )
                .await?;
            }
            None => {
                let (walk, mut params) = if plan.has_match {
                    (Walk::Matched, plan.params_for(Form::Driven))
                } else {
                    (Walk::Messages, plan.params.clone())
                };
                // Twice for the union's two halves, and room for every
                // refused row on top: the cap counts what is kept.
                params.push(turso::Value::Integer(
                    (cap() + refuse.len() as i64).saturating_mul(2),
                ));
                sql::each(connection, &projection_sql(plan, walk), params, |row| {
                    let id: i64 = row.col(0)?;
                    if refuse.contains(&id) {
                        return Ok(true);
                    }
                    fold.take(row, None)
                })
                .await?;
            }
        }
        Ok(fold)
    }

    /// One row of the projection; answers whether to read on. `scores` are
    /// the message's text score and whether its body matched, when they
    /// were read apart from the row ([`Fold::walk_sets`]).
    fn take(&mut self, row: &turso::Row, scores: Option<Scores>) -> postio_storage::Result<bool> {
        let id: i64 = row.col(0)?;
        let scores = match scores {
            Some(scores) => scores,
            None => {
                let meta: Option<f64> = row.col(9)?;
                let body: Option<f64> = row.col(10)?;
                let file: Option<f64> = row.col(16)?;
                Scores {
                    text: meta.unwrap_or(0.0)
                        + BODY_SCORE_WEIGHT * (body.unwrap_or(0.0) + file.unwrap_or(0.0)),
                    in_body: body.is_some(),
                    in_file: file.is_some(),
                }
            }
        };
        if let Some(at) = self.at.get(&id) {
            // The same message from another index: the halves add, as
            // `fetch_candidates` adds them.
            let found = &mut self.found[*at];
            found.text += scores.text;
            found.in_body |= scores.in_body;
            found.in_file |= scores.in_file;
            return Ok(true);
        }
        if self.passed.contains(&id) {
            return Ok(true);
        }
        // Another occurrence of a content already held: the earliest stands
        // for it, as `search`'s `where_sql` picks it. Every row the walk
        // hands over already passed the plan's conditions and sets, so the
        // earliest met here is the earliest that qualifies. The text score
        // is the content's, whichever occurrence carried it.
        let content: Option<i64> = row.col(15)?;
        if let Some(at) = content.and_then(|content| self.contents.get(&content).copied()) {
            let held = self.found[at].id;
            if id > held {
                self.passed.insert(id);
                return Ok(true);
            }
            let held_scores = Scores {
                text: self.found[at].text,
                in_body: self.found[at].in_body,
                in_file: self.found[at].in_file,
            };
            self.found[at] = Self::found(row, id, held_scores)?;
            self.at.remove(&held);
            self.at.insert(id, at);
            self.passed.insert(held);
            return Ok(true);
        }
        if self.capped() {
            // The cap's worth of messages is in hand. A message met in the
            // metadata half and not yet in the body half keeps the one score
            // it has: past the cap, the counts are floors and the order is
            // approximate, as `search`'s is.
            return Ok(false);
        }
        if let Some(content) = content {
            self.contents.insert(content, self.found.len());
        }
        self.at.insert(id, self.found.len());
        let found = Self::found(row, id, scores)?;
        self.found.push(found);
        Ok(true)
    }

    /// One projection row as a match, scored as `scores` say.
    fn found(row: &turso::Row, id: i64, scores: Scores) -> postio_storage::Result<Found> {
        let thread: Option<i64> = row.col(1)?;
        let (senders, recipients) = people(row.col::<Option<String>>(11)?.as_deref());
        Ok(Found {
            id,
            key: match thread {
                Some(thread) => ConversationKey::Thread(postio_model::ThreadId::new(thread)),
                None => ConversationKey::Lone(MessageId::new(id)),
            },
            mailbox: row.col(2)?,
            received_at: row.col(3)?,
            aged_from: row.col(4)?,
            unread: !row.col::<bool>(5)?,
            flagged: row.col(6)?,
            answered: row.col(7)?,
            attachment: row.col(8)?,
            in_body: scores.in_body,
            in_file: scores.in_file,
            text: scores.text,
            senders,
            recipients,
            labels: ids(row.col::<Option<String>>(12)?.as_deref()),
            action: row.col(13)?,
            files: row.col::<i64>(14)?.max(0) as u64,
        })
    }

    /// Whether the match reached the cap: every count is then a floor.
    fn capped(&self) -> bool {
        self.found.len() as u64 >= TOTAL_HITS_CAP
    }

    /// The matched messages grouped into conversations, in the order each
    /// was first met.
    fn conversations(&self) -> Vec<Conversation> {
        let mut index: HashMap<ConversationKey, usize> = HashMap::new();
        let mut conversations: Vec<Conversation> = Vec::new();
        for (at, found) in self.found.iter().enumerate() {
            let slot = *index.entry(found.key).or_insert_with(|| {
                conversations.push(Conversation::new(found.key, at));
                conversations.len() - 1
            });
            conversations[slot].add(&self.found, at);
        }
        for conversation in &mut conversations {
            for set in [
                &mut conversation.senders,
                &mut conversation.recipients,
                &mut conversation.labels,
                &mut conversation.folders,
            ] {
                set.sort_unstable();
                set.dedup();
            }
        }
        conversations
    }
}

/// The narrow projection (research R2): the capped match, one row per
/// message per index it matched in.
///
/// Every column but the scores is the message's own or one short lookup by
/// its id: its people and labels as `group_concat`s over the
/// `(message_id, …)` keys of `recipients` and `message_labels`, an open
/// marker by `markers`' primary key, and its attachments, asked only when
/// it has any. A sender is spelled negated, so one lookup answers both the
/// From and the To facet.
///
/// The scores are projected bare and negated outside, for the reason
/// [`HITS_JOIN`](super::HITS_JOIN) gives.
fn projection_sql(plan: &Plan, walk: Walk) -> String {
    let (scores, from, where_sql) = match walk {
        Walk::Matched => (
            ["-hits.meta, -hits.body", "-hits.file"],
            HITS_JOIN_WITH_FILES.to_owned(),
            plan.where_sql(Form::Driven),
        ),
        Walk::Messages => (
            ["NULL, NULL", "NULL"],
            "FROM messages m".to_owned(),
            plan.conditions.join(" AND "),
        ),
        Walk::Keyed => (
            ["NULL, NULL", "NULL"],
            "FROM json_each(?) j CROSS JOIN messages m ON m.id = j.value".to_owned(),
            plan.conditions.join(" AND "),
        ),
    };
    let [scores, file] = scores;
    format!(
        "SELECT m.id, m.thread_id, m.mailbox_id, m.received_at, {AGED_FROM},
                m.seen, m.flagged, m.answered, m.has_attachments, {scores},
                (SELECT group_concat(CASE r.kind WHEN 'from' THEN -r.address_id
                                                 ELSE r.address_id END)
                   FROM recipients r
                  WHERE r.message_id = m.id AND r.kind IN ('from', 'to', 'cc', 'bcc')),
                (SELECT group_concat(ml.label_id) FROM message_labels ml
                  WHERE ml.message_id = m.id),
                EXISTS (SELECT 1 FROM markers k
                         WHERE k.message_id = m.id AND k.dismissed_at IS NULL),
                CASE WHEN m.has_attachments = 1
                     THEN (SELECT count(*) FROM attachments f WHERE f.message_id = m.id)
                     ELSE 0 END,
                m.content_id, {file}
           {from}
          WHERE {where_sql}
          LIMIT ?",
    )
}

/// What the projection walks.
#[derive(Debug, Clone, Copy)]
enum Walk {
    /// The free-text match, joined to `messages`
    /// ([`super::HITS_JOIN_WITH_FILES`]).
    Matched,
    /// `messages` under the plan's conditions.
    Messages,
    /// The messages a JSON array of ids names, each by its key.
    Keyed,
}

/// The metadata index's five columns, as every `fts_match` on it names them.
const META: &str = "sender, recipients, subject, filenames, list_id";

/// [`TOTAL_HITS_CAP`], as SQL binds it.
fn cap() -> i64 {
    i64::try_from(TOTAL_HITS_CAP).unwrap_or(i64::MAX)
}

/// A `group_concat` of integers, read back.
fn ids(list: Option<&str>) -> Vec<i64> {
    list.map(|list| {
        list.split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect()
    })
    .unwrap_or_default()
}

/// The people column: senders negated, recipients as they are.
fn people(list: Option<&str>) -> (Vec<i64>, Vec<i64>) {
    let (senders, recipients): (Vec<i64>, Vec<i64>) = ids(list).into_iter().partition(|id| *id < 0);
    (senders.into_iter().map(|id| -id).collect(), recipients)
}

/// One conversation of the match.
#[derive(Debug, Clone)]
struct Conversation {
    key: ConversationKey,
    /// Its best match, an index into [`Fold::found`].
    best: usize,
    /// When its newest match arrived, in milliseconds.
    newest: i64,
    matches: u32,
    unread: bool,
    flagged: bool,
    answered: bool,
    attachment: bool,
    action: bool,
    files: u64,
    senders: Vec<i64>,
    recipients: Vec<i64>,
    labels: Vec<i64>,
    folders: Vec<i64>,
}

impl Conversation {
    fn new(key: ConversationKey, first: usize) -> Self {
        Self {
            key,
            best: first,
            newest: i64::MIN,
            matches: 0,
            unread: false,
            flagged: false,
            answered: false,
            attachment: false,
            action: false,
            files: 0,
            senders: Vec::new(),
            recipients: Vec::new(),
            labels: Vec::new(),
            folders: Vec::new(),
        }
    }

    fn add(&mut self, found: &[Found], at: usize) {
        let message = &found[at];
        let best = &found[self.best];
        // The best text match, then the newer, then the later id: a total
        // order, so the best message does not depend on the walk.
        if message
            .text
            .total_cmp(&best.text)
            .then(best.received_at.cmp(&message.received_at))
            .then(best.id.cmp(&message.id))
            .is_lt()
        {
            self.best = at;
        }
        self.newest = self.newest.max(message.received_at);
        self.matches += 1;
        self.unread |= message.unread;
        self.flagged |= message.flagged;
        self.answered |= message.answered;
        self.attachment |= message.attachment;
        self.action |= message.action;
        self.files += message.files;
        self.senders.extend(&message.senders);
        self.recipients.extend(&message.recipients);
        self.labels.extend(&message.labels);
        self.folders.push(message.mailbox);
    }
}

// ---------------------------------------------------------------------------
// The facets
// ---------------------------------------------------------------------------

/// How many senders and recipients a facet keeps: the popovers' lists.
const FACET_PEOPLE: usize = 50;

/// Every facet, counted in conversations over the fold (D3): a conversation
/// counts once for a value when any of its matched messages has it, which
/// is exactly when the query with that term added would still find it
/// (SC-008). The months put a conversation in the month of its newest
/// match (D4), and a preset counts it when its newest match is on or after
/// the preset's start, which is when that `after:` would find it.
fn facets(conversations: &[Conversation], today: NaiveDate, capped: bool) -> SearchFacets {
    let mut senders: HashMap<i64, u64> = HashMap::new();
    let mut recipients: HashMap<i64, u64> = HashMap::new();
    let mut labels: HashMap<i64, u64> = HashMap::new();
    let mut folders: HashMap<i64, u64> = HashMap::new();
    let mut facets = SearchFacets {
        capped,
        ..SearchFacets::default()
    };

    let firsts = months_ending(today);
    let bounds: Vec<i64> = firsts
        .iter()
        .copied()
        .chain(firsts[11].checked_add_months(chrono::Months::new(1)))
        .map(super::day_start_millis)
        .collect();
    let presets: Vec<Option<i64>> = preset_starts(today)
        .iter()
        .map(|start| start.map(super::day_start_millis))
        .collect();
    let mut months = [0u64; 12];

    for conversation in conversations {
        for (set, counts) in [
            (&conversation.senders, &mut senders),
            (&conversation.recipients, &mut recipients),
            (&conversation.labels, &mut labels),
            (&conversation.folders, &mut folders),
        ] {
            for id in set {
                *counts.entry(*id).or_default() += 1;
            }
        }
        facets.attachment += u64::from(conversation.attachment);
        facets.action += u64::from(conversation.action);
        facets.unread += u64::from(conversation.unread);
        let newest = conversation.newest;
        if let Some(month) = bounds
            .windows(2)
            .position(|bound| bound[0] <= newest && newest < bound[1])
        {
            months[month] += 1;
        }
        for (count, start) in facets.presets.iter_mut().zip(&presets) {
            if start.is_none_or(|start| newest >= start) {
                *count += 1;
            }
        }
    }

    facets.senders = ranked(senders, Some(FACET_PEOPLE), AddressId::new);
    facets.recipients = ranked(recipients, Some(FACET_PEOPLE), AddressId::new);
    facets.labels = ranked(labels, None, LabelId::new);
    facets.folders = ranked(folders, None, MailboxId::new);
    facets.months = std::array::from_fn(|at| MonthCount {
        month: firsts[at],
        conversations: months[at],
    });
    facets
}

/// A facet's counts, most first, ties by id so the order is stable.
fn ranked<T>(counts: HashMap<i64, u64>, keep: Option<usize>, id: fn(i64) -> T) -> Vec<Count<T>> {
    let mut counts: Vec<(i64, u64)> = counts.into_iter().collect();
    counts.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    if let Some(keep) = keep {
        counts.truncate(keep);
    }
    counts
        .into_iter()
        .map(|(value, conversations)| Count {
            id: id(value),
            conversations,
        })
        .collect()
}

/// Everyone the matched messages are from or to: the People tab's count.
fn people_in(conversations: &[Conversation]) -> u64 {
    let mut everyone: Vec<i64> = conversations
        .iter()
        .flat_map(|conversation| conversation.senders.iter().chain(&conversation.recipients))
        .copied()
        .collect();
    everyone.sort_unstable();
    everyone.dedup();
    everyone.len() as u64
}

// ---------------------------------------------------------------------------
// The order
// ---------------------------------------------------------------------------

/// The conversations in their order, before and after the pool is ranked.
struct Order {
    /// Every conversation, by the pool ordering (or by date, for Newest).
    ranked: Vec<usize>,
    /// The ones ranked in full: the head of `ranked`, empty for Newest.
    pool: Vec<usize>,
}

impl Order {
    fn len(&self) -> usize {
        self.ranked.len()
    }

    /// The final order: the pool by [`relevance_keys`], then the rest as
    /// they were.
    fn rank(
        self,
        fold: &Fold,
        conversations: &[Conversation],
        rows: &HashMap<i64, Hydrated>,
        query: &ParsedQuery,
        now: DateTime<Utc>,
    ) -> Vec<usize> {
        if self.pool.is_empty() {
            return self.ranked;
        }
        let mut candidates: Vec<Candidate> = self
            .pool
            .iter()
            .map(|at| candidate(fold, &conversations[*at], rows))
            .collect();
        let keys = relevance_keys(&mut candidates, query, now);
        let mut keyed: Vec<(super::RelevanceKey, usize)> =
            keys.into_iter().zip(self.pool.iter().copied()).collect();
        keyed.sort_by(|(a, a_at), (b, b_at)| {
            let more = conversations[*b_at]
                .matches
                .cmp(&conversations[*a_at].matches);
            a.order(b, more)
        });
        let mut order: Vec<usize> = keyed.into_iter().map(|(_, at)| at).collect();
        order.extend(self.ranked.into_iter().skip(self.pool.len()));
        order
    }
}

/// The order before the pool is ranked.
fn order(
    fold: &Fold,
    conversations: &[Conversation],
    order: ConversationOrder,
    now: DateTime<Utc>,
) -> Order {
    let mut ranked: Vec<usize> = (0..conversations.len()).collect();
    let best = |at: usize| &fold.found[conversations[at].best];
    match order {
        ConversationOrder::Newest => {
            ranked.sort_by(|a, b| {
                conversations[*b]
                    .newest
                    .cmp(&conversations[*a].newest)
                    .then(best(*b).id.cmp(&best(*a).id))
            });
            Order {
                ranked,
                pool: Vec::new(),
            }
        }
        ConversationOrder::BestMatch => {
            let keys: Vec<f64> = (0..conversations.len())
                .map(|at| pool_key(best(at), now))
                .collect();
            ranked.sort_by(|a, b| {
                keys[*a]
                    .total_cmp(&keys[*b])
                    .then(conversations[*b].newest.cmp(&conversations[*a].newest))
                    .then(best(*b).id.cmp(&best(*a).id))
            });
            let pool = ranked[..ranked.len().min(CONVERSATION_POOL)].to_vec();
            Order { ranked, pool }
        }
    }
}

/// The pool ordering `fetch_candidates` writes in SQL, here in Rust: the
/// text score with a linear age term, lower first.
fn pool_key(best: &Found, now: DateTime<Utc>) -> f64 {
    let age = (now.timestamp_millis() - best.aged_from) as f64;
    best.text + POOL_AGE_WEIGHT_PER_YEAR * age / MILLIS_PER_YEAR
}

/// A conversation's best message, as the ranking reads a candidate.
fn candidate(fold: &Fold, conversation: &Conversation, rows: &HashMap<i64, Hydrated>) -> Candidate {
    let best = &fold.found[conversation.best];
    let row = rows.get(&best.id);
    Candidate {
        message_id: MessageId::new(best.id),
        thread_id: match conversation.key {
            ConversationKey::Thread(thread) => Some(thread),
            ConversationKey::Lone(_) => None,
        },
        mailbox_id: MailboxId::new(best.mailbox),
        subject: row.and_then(|row| row.subject.clone()),
        from_name: row.and_then(|row| row.from_name.clone()),
        from_address: row.and_then(|row| row.from_address.clone()),
        received_at: from_millis(best.received_at),
        aged_from: from_millis(best.aged_from),
        preview: None,
        snippet: String::new(),
        bm25: best.text,
        sender_times_seen: row.map_or(0, |row| row.times_seen),
        score: 0.0,
    }
}

// ---------------------------------------------------------------------------
// The rows shown
// ---------------------------------------------------------------------------

/// What the hydrate adds to a best message.
#[derive(Debug, Clone, Default)]
struct Hydrated {
    subject: Option<String>,
    from_name: Option<String>,
    from_address: Option<String>,
    times_seen: i64,
    sent_count: i64,
    /// The thread's own counts, when it has one.
    thread: Option<(u32, bool, bool)>,
    files: Vec<(AttachmentId, String)>,
}

/// The separator between a hydrated row's attachments: a unit separator,
/// which no file name carries.
const UNIT: char = '\u{1f}';

/// One read for every message that is ranked or shown: its subject and
/// sender, how often the person hears from and writes to that sender, its
/// thread's counts, and its attachments' names.
///
/// The sender's `recipients` row is found once and joined by primary key,
/// and the contacts probe compares against a plain column, as
/// [`Plan::hydrate_sql`](super::Plan) does and for the reasons it gives.
async fn hydrate(
    connection: &Connection,
    account: AccountScope,
    ids: &[i64],
) -> Result<HashMap<i64, Hydrated>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let affinity = match account.account() {
        Some(_) => "AND (c.account_id = ? OR c.account_id IS NULL)",
        None => "",
    };
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT sub.id, sub.subject, sender.name, a.address,
                (SELECT max(c.times_seen) FROM contacts c
                  WHERE c.address_normalized = a.address_normalized {affinity}),
                (SELECT w.sent_count FROM correspondents w WHERE w.address_id = a.id),
                sub.thread_messages, sub.thread_unread, sub.thread_files, sub.files
           FROM (SELECT m.id, m.subject,
                        (SELECT r.id FROM recipients r
                          WHERE r.message_id = m.id AND r.kind = 'from'
                          ORDER BY r.position LIMIT 1) AS from_recipient,
                        t.message_count AS thread_messages,
                        t.unread_count AS thread_unread,
                        t.has_attachments AS thread_files,
                        (SELECT group_concat(f.id || ':' || f.filename, '{UNIT}')
                           FROM attachments f
                          WHERE f.message_id = m.id AND f.filename IS NOT NULL) AS files
                   FROM messages m LEFT JOIN threads t ON t.id = m.thread_id
                  WHERE m.id IN ({placeholders})) sub
           LEFT JOIN recipients sender ON sender.id = sub.from_recipient
           LEFT JOIN addresses a ON a.id = sender.address_id",
    );
    let mut params = Vec::with_capacity(ids.len() + 1);
    if let Some(id) = account.account() {
        params.push(turso::Value::Integer(id.get()));
    }
    params.extend(ids.iter().map(|id| turso::Value::Integer(*id)));

    let mut statement = connection.prepare(&sql).await?;
    let rows = sql::mapped(&mut statement, params, |row| {
        let thread_messages: Option<i64> = row.col(6)?;
        Ok((
            row.col::<i64>(0)?,
            Hydrated {
                subject: row.col(1)?,
                from_name: row.col(2)?,
                from_address: row.col(3)?,
                times_seen: row.col::<Option<i64>>(4)?.unwrap_or(0),
                sent_count: row.col::<Option<i64>>(5)?.unwrap_or(0),
                thread: match thread_messages {
                    Some(messages) => Some((
                        u32::try_from(messages.max(1)).unwrap_or(u32::MAX),
                        row.col::<Option<i64>>(7)?.unwrap_or(0) > 0,
                        row.col::<Option<bool>>(8)?.unwrap_or(false),
                    )),
                    None => None,
                },
                files: row
                    .col::<Option<String>>(9)?
                    .map(|files| {
                        files
                            .split(UNIT)
                            .filter_map(|file| {
                                let (id, name) = file.split_once(':')?;
                                Some((AttachmentId::new(id.parse().ok()?), name.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
        ))
    })
    .await?;
    Ok(rows.into_iter().collect())
}

/// The words a subject or a file name is checked for: the free text asked
/// for, and the values of the operators about that field.
struct Terms {
    subject: Vec<String>,
    files: Vec<String>,
}

impl Terms {
    fn of(query: &ParsedQuery) -> Self {
        let words: Vec<String> = query
            .searchable_terms()
            .filter(|term| !term.negated)
            .map(|term| term.value.clone())
            .collect();
        let named = |field: fn(&Filter) -> Option<&String>| {
            let mut terms = words.clone();
            terms.extend(
                query
                    .filters()
                    .filter(|clause| !clause.negated)
                    // Every value of a set (D26).
                    .flat_map(|clause| clause.filter.alternatives())
                    .filter_map(|filter| field(filter).cloned()),
            );
            terms
        };
        Self {
            subject: named(|filter| match filter {
                Filter::Subject(value) => Some(value),
                _ => None,
            }),
            files: named(|filter| match filter {
                Filter::Filename(value) => Some(value),
                _ => None,
            }),
        }
    }
}

/// One row of the page.
fn hit(
    fold: &Fold,
    conversation: &Conversation,
    rows: &HashMap<i64, Hydrated>,
    files: &[super::FileMatch],
    terms: &Terms,
    now: DateTime<Utc>,
) -> ConversationHit {
    let best = &fold.found[conversation.best];
    let row = rows.get(&best.id).cloned().unwrap_or_default();
    let when = Some(from_millis(best.received_at));

    // Where the best message matched. The body is known from which index
    // matched it; a subject or a file name from whether the words are in
    // it, as the row will highlight them (`highlight::find`), so the reason
    // and the highlight cannot disagree.
    let mut matches = Vec::new();
    let in_subject = row
        .subject
        .as_deref()
        .is_some_and(|subject| !postio_search::highlight::find(subject, &terms.subject).is_empty());
    if in_subject {
        matches.push(Match {
            source: Source::Subject,
            passage: None,
            when,
        });
    }
    if best.in_body {
        matches.push(Match {
            source: Source::Body,
            passage: None,
            when,
        });
    }
    let named: Vec<&(AttachmentId, String)> = row
        .files
        .iter()
        .filter(|(_, name)| !postio_search::highlight::find(name, &terms.files).is_empty())
        .collect();
    for (attachment, name) in &named {
        matches.push(Match {
            source: Source::FileName {
                attachment: *attachment,
                name: name.clone(),
            },
            passage: None,
            when,
        });
    }
    // What its attachments say: where in each, the first unit that
    // matched. Its passage is cut with the others (`Req::Passages`).
    let read: Vec<&super::FileMatch> = files
        .iter()
        .filter(|file| file.message.get() == best.id)
        .collect();
    for file in &read {
        matches.push(Match {
            source: Source::FileContent {
                attachment: file.attachment,
                name: file.name.clone(),
                location: file.location.clone(),
            },
            passage: None,
            when,
        });
    }

    let mut reasons = Vec::new();
    if conversation.answered {
        reasons.push(RankReason::Replied);
    }
    if conversation.flagged {
        reasons.push(RankReason::Flagged);
    }
    if row.times_seen + row.sent_count >= FREQUENT_SENDER {
        reasons.push(RankReason::FrequentSender);
    }
    if in_subject {
        reasons.push(RankReason::InSubject);
    }
    // A file found it, by its name or by what it says (research R3).
    if !named.is_empty() || !read.is_empty() {
        reasons.push(RankReason::InFileName);
    }
    reasons.push(RankReason::Matches(conversation.matches));

    let (messages, unread, has_attachments) = match (conversation.key, row.thread) {
        (ConversationKey::Thread(_), Some(thread)) => thread,
        _ => (1, best.unread, best.attachment),
    };
    ConversationHit {
        key: conversation.key,
        best: MessageId::new(best.id),
        mailbox_id: MailboxId::new(best.mailbox),
        subject: row.subject.clone(),
        from: row
            .from_address
            .clone()
            .map(|address| EmailAddress::new(row.from_name.clone(), address)),
        newest_match: from_millis(conversation.newest),
        messages,
        unread,
        has_attachments,
        labels: conversation
            .labels
            .iter()
            .map(|id| LabelId::new(*id))
            .collect(),
        score: pool_key(best, now),
        reasons,
        matches,
    }
}

/// What a search looked through.
struct Searched {
    /// Messages in its scope.
    messages: u64,
    /// Every one of them has its body.
    bodies: bool,
    /// Every attachment on this machine has had its text read.
    contents: bool,
}

/// How many messages the search looked through, whether every one of them
/// has its body, and whether every downloaded attachment has been read:
/// one read of the folders in its scope, the same scope the match uses,
/// off the counts the schema keeps (`total_count`, `bodies_owed`; see
/// [`corpus_complete`](super::corpus_complete)), with the indexer's own
/// question about attachments ([`crate::index::attachments_missing_text`])
/// asked once beside it.
///
/// The attachments are every account's, as the indexer's queue is: the
/// search cannot say it looked inside files while any file this machine
/// holds is still unread.
async fn searched(connection: &Connection, request: &ConversationRequest<'_>) -> Result<Searched> {
    let mut conditions = Vec::new();
    // The unread attachment's version first: its `?` comes first in the
    // statement.
    let mut params: Vec<turso::Value> = vec![turso::Value::Integer(i64::from(
        postio_extract::EXTRACTOR_VERSION,
    ))];
    match request.account.account() {
        Some(id) => {
            conditions.push("account_id = ?".to_owned());
            params.push(turso::Value::Integer(id.get()));
        }
        None => conditions.push(
            "account_id IN (SELECT id FROM accounts WHERE enabled = 1 AND pending_deletion = 0)"
                .to_owned(),
        ),
    }
    if let Some(role) = scope_role(Scope::AllMail, names_a_folder(request.query)) {
        conditions.push(role.to_owned());
    }
    let (total, owed, unread): (i64, i64, bool) = sql::one(
        connection,
        &format!(
            "SELECT coalesce(sum(total_count), 0), coalesce(max(bodies_owed > 0), 0),
                    EXISTS ({})
               FROM mailboxes WHERE {}",
            crate::index::MISSING,
            conditions.join(" AND ")
        ),
        params,
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
    )
    .await?;
    Ok(Searched {
        messages: total.max(0) as u64,
        bodies: owed == 0,
        contents: !unread,
    })
}
