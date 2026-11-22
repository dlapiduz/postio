//! Conversation search: the match answered as conversations with every facet and the months
//! histogram, in a fixed number of statements.
//! The match is read once as a narrow projection and folded in Rust, stopping at
//! [`CONVERSATION_WALK_CAP`] messages; a filter that is a set of messages is read once as ids,
//! since `m.id IN (…)` tested per row took 440 ms on 20,000 messages and 6 ms as a set. The order
//! never depends on the page asked for, so paging cannot reorder what was shown.

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
    CONVERSATION_WALK_CAP, ConversationHit, ConversationKey, ConversationOrder,
    ConversationResults, FacetNames, Match, RankReason, Source,
};
use postio_storage::Connection;
use postio_storage::repository::from_millis;
use postio_storage::sql::{self, RowExt as _};

use super::{
    AGED_FROM, BODY_SCORE_WEIGHT, COPY_KEY, Candidate, Form, HITS_JOIN_WITH_FILES, MILLIS_PER_YEAR,
    POOL_AGE_WEIGHT_PER_YEAR, Plan, SearchRequest, names_a_folder, relevance_keys, scope_role,
};
use crate::error::Result;

/// A constant, not a multiple of the page: the order must not depend on the page asked for.
const CONVERSATION_POOL: usize = 250;

const FREQUENT_SENDER: i64 = 10;

#[derive(Debug, Clone, Copy)]
pub struct ConversationRequest<'a> {
    pub account: AccountScope,
    pub query: &'a ParsedQuery,
    pub order: ConversationOrder,
    pub offset: u32,
    pub limit: u32,
    pub today: NaiveDate,
}

impl ConversationRequest<'_> {
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

/// The query's alone, never the page asked for, so the dropdown's "Show all N" and the results'
/// count are of one list (ADR 0037).
const FORGIVE_BELOW: usize = 50;

/// The same for the search, its Files tab and its People tab, so their counts are of one match.
async fn walked(
    connection: &Connection,
    request: &ConversationRequest<'_>,
) -> Result<(Fold, Option<std::collections::HashSet<ConversationKey>>)> {
    let search = request.as_search();
    let exact = walk(connection, &Plan::build_sets(&search)).await?;
    if !request.query.is_forgiving() {
        return Ok((exact, None));
    }
    let held = exact.conversations();
    if held.len() >= FORGIVE_BELOW {
        return Ok((exact, None));
    }
    let words = super::near_words(connection, &search).await?;
    if words.is_empty() {
        return Ok((exact, None));
    }
    let near = walk(connection, &Plan::build_sets_near(&search, &words)).await?;
    Ok((
        near,
        Some(
            held.into_iter()
                .map(|conversation| conversation.key)
                .collect(),
        ),
    ))
}

async fn walk(connection: &Connection, plan: &Plan) -> Result<Fold> {
    if plan.sets.is_empty() {
        Fold::walk(connection, plan).await
    } else {
        Fold::walk_sets(connection, plan).await
    }
}

pub async fn search_conversations(
    connection: &Connection,
    request: &ConversationRequest<'_>,
    now: DateTime<Utc>,
) -> Result<ConversationResults> {
    let start = Instant::now();
    let (fold, exact) = walked(connection, request).await?;
    let conversations = fold.conversations();
    // Exact matches come before what only the near words found (ADR 0037).
    let exact: Vec<bool> = conversations
        .iter()
        .map(|conversation| {
            exact
                .as_ref()
                .is_none_or(|keys| keys.contains(&conversation.key))
        })
        .collect();

    let order = order(&fold, &conversations, &exact, request.order, now);
    let offset = (request.offset as usize).min(order.len());
    let end = offset
        .saturating_add(request.limit as usize)
        .min(order.len());
    let page = &order.ranked[offset..end];

    let mut wanted: Vec<i64> = order
        .pool
        .iter()
        .map(|at| fold.found[conversations[*at].best].id)
        .collect();
    wanted.extend(page.iter().map(|at| fold.found[conversations[*at].best].id));
    wanted.sort_unstable();
    wanted.dedup();
    let rows = hydrate(connection, request.account, &wanted).await?;
    let order = order.rank(&fold, &conversations, &exact, &rows, request.query, now);
    let page: Vec<usize> = order[offset..end].to_vec();

    let in_files: Vec<MessageId> = page
        .iter()
        .map(|at| &fold.found[conversations[*at].best])
        .filter(|found| found.in_file)
        .map(|found| MessageId::new(found.id))
        .collect();
    let in_file: Vec<MessageId> = fold
        .found
        .iter()
        .filter(|found| found.in_file)
        .map(|found| MessageId::new(found.id))
        .collect();
    let (files, cards) = super::files::page_and_cards(
        connection,
        request.query,
        &carried(&fold),
        &in_file,
        &in_files,
    )
    .await?;

    let terms = Terms::of(request.query);
    let hits: Vec<ConversationHit> = page
        .iter()
        .map(|at| hit(&fold, &conversations[*at], &rows, &files, &terms, now))
        .collect();

    let searched = searched(connection, request).await?;
    let elapsed = start.elapsed();
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
        files: cards,
        people: people_in(&conversations, &searched.own),
        names: FacetNames::default(),
        elapsed,
    })
}

pub(super) struct Carrier {
    pub(super) id: MessageId,
    pub(super) in_file: bool,
}

pub(super) async fn carriers(
    connection: &Connection,
    request: &ConversationRequest<'_>,
) -> Result<Vec<Carrier>> {
    let (fold, _) = walked(connection, request).await?;
    Ok(carried(&fold))
}

fn carried(fold: &Fold) -> Vec<Carrier> {
    fold.found
        .iter()
        .filter(|found| found.files > 0)
        .map(|found| Carrier {
            id: MessageId::new(found.id),
            in_file: found.in_file,
        })
        .collect()
}

pub(super) struct Correspondent {
    pub(super) address: i64,
    pub(super) from: u64,
    pub(super) to: u64,
    pub(super) last: i64,
}

pub(super) async fn correspondents(
    connection: &Connection,
    request: &ConversationRequest<'_>,
) -> Result<Vec<Correspondent>> {
    let (fold, _) = walked(connection, request).await?;
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

pub(super) fn file_terms(query: &ParsedQuery) -> Vec<String> {
    Terms::of(query).files
}

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
    files: u64,
    text: f64,
    in_body: bool,
    in_file: bool,
    senders: Vec<i64>,
    recipients: Vec<i64>,
    labels: Vec<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Scores {
    text: f64,
    in_body: bool,
    in_file: bool,
}

#[derive(Debug, Default)]
struct Fold {
    found: Vec<Found>,
    at: HashMap<i64, usize>,
    /// A message filed in two folders is one content (#1780), and one match.
    contents: HashMap<i64, usize>,
    /// An IMAP message filed in two folders has two contents and is one match all the same (#1526).
    copies: HashMap<String, usize>,
    passed: std::collections::HashSet<i64>,
}

impl Fold {
    async fn walk(connection: &Connection, plan: &Plan) -> Result<Self> {
        let mut fold = Fold::default();
        let mut params = plan.params_for(Form::Driven);
        // Rows, not messages: the union hands a message back once per index it matched in.
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

    /// Two statements where [`Fold::walk`] is one, far cheaper with a set: `budget from:ada` reads
    /// two id lists and walks `messages` only for the few in both.
    async fn walk_sets(connection: &Connection, plan: &Plan) -> Result<Self> {
        // Free text is read here only when a set must also hold; with refusals alone the walk is
        // the match itself and a refused message is skipped as it streams past.
        let keeps = plan.sets.iter().any(|set| !set.negated);
        let read_text = plan.has_match && keeps;
        let mut arms: Vec<String> = Vec::new();
        let mut params: Vec<turso::Value> = Vec::new();
        if read_text {
            // `fts_score` bare with the match's own parameter, as `HITS_JOIN` explains. Both
            // indexes are keyed by content: each match is joined to every message carrying it.
            arms.push(format!(
                "SELECT -2, m.id, h.meta, NULL, NULL
                   FROM (SELECT content_id, fts_score({META}, ?1) AS meta
                           FROM search_documents WHERE fts_match({META}, ?1)) h
                   CROSS JOIN messages m INDEXED BY idx_messages_content
                     ON m.content_id = h.content_id"
            ));
            arms.push(
                "SELECT -1, m.id, NULL, h.body, NULL
                   FROM (SELECT content_id, fts_score(body_search, ?2) AS body
                           FROM message_search_bodies WHERE fts_match(body_search, ?2)) h
                   CROSS JOIN messages m INDEXED BY idx_messages_content
                     ON m.content_id = h.content_id"
                    .to_owned(),
            );
            arms.push(
                "SELECT -3, m.id, NULL, NULL, h.file
                   FROM (SELECT content_id, max(unit_score) AS file
                           FROM (SELECT content_id, fts_score(text_search, ?2) AS unit_score
                                   FROM attachment_passages WHERE fts_match(text_search, ?2))
                          GROUP BY content_id) h
                   CROSS JOIN messages m INDEXED BY idx_messages_content
                     ON m.content_id = h.content_id"
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
                // Twice for the union's two halves, plus every refused row: the cap counts what is
                // kept.
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
            let found = &mut self.found[*at];
            found.text += scores.text;
            found.in_body |= scores.in_body;
            found.in_file |= scores.in_file;
            return Ok(true);
        }
        if self.passed.contains(&id) {
            return Ok(true);
        }
        // Another occurrence of a content already held: the earliest stands for it, as `search`'s
        // `where_sql` picks it.
        // An exact copy stands the same way; only rows that matched reach the fold, so the copy
        // that stands always matched.
        let content: Option<i64> = row.col(15)?;
        let copy: Option<String> = row.col(17)?;
        let held_at = content
            .and_then(|content| self.contents.get(&content).copied())
            .or_else(|| {
                copy.as_ref()
                    .and_then(|copy| self.copies.get(copy).copied())
            });
        if let Some(at) = held_at {
            if let Some(content) = content {
                self.contents.insert(content, at);
            }
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
            // Past the cap the counts are floors and the order approximate, as `search`'s is: a
            // message met in one half only keeps the one score it has.
            return Ok(false);
        }
        if let Some(content) = content {
            self.contents.insert(content, self.found.len());
        }
        if let Some(copy) = copy {
            self.copies.insert(copy, self.found.len());
        }
        self.at.insert(id, self.found.len());
        let found = Self::found(row, id, scores)?;
        self.found.push(found);
        Ok(true)
    }

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

    fn capped(&self) -> bool {
        self.found.len() as u64 >= CONVERSATION_WALK_CAP
    }

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

/// One short lookup by id for the open marker; people, labels and attachment count are trigger-kept
/// columns of `messages` (D30), where they were three lookups a row.
/// A sender is spelled negated, so one column answers both the From and the To facet. Scores are
/// projected bare and negated outside ([`HITS_JOIN`](super::HITS_JOIN)). `driven_join_plan` holds
/// that no lookup comes back.
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
                m.people_ids, m.label_ids,
                EXISTS (SELECT 1 FROM markers k
                         WHERE k.message_id = m.id AND k.dismissed_at IS NULL),
                CASE WHEN m.has_attachments = 1 THEN m.attachment_count ELSE 0 END,
                m.content_id, {file}, {COPY_KEY}
           {from}
          WHERE {where_sql}
          LIMIT ?",
    )
}

#[derive(Debug, Clone, Copy)]
enum Walk {
    Matched,
    Messages,
    Keyed,
}

const META: &str = "sender, recipients, subject, filenames, list_id";

fn cap() -> i64 {
    i64::try_from(CONVERSATION_WALK_CAP).unwrap_or(i64::MAX)
}

fn ids(list: Option<&str>) -> Vec<i64> {
    list.map(|list| {
        list.split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect()
    })
    .unwrap_or_default()
}

fn people(list: Option<&str>) -> (Vec<i64>, Vec<i64>) {
    let (senders, recipients): (Vec<i64>, Vec<i64>) = ids(list).into_iter().partition(|id| *id < 0);
    (senders.into_iter().map(|id| -id).collect(), recipients)
}

#[derive(Debug, Clone)]
struct Conversation {
    key: ConversationKey,
    best: usize,
    newest: i64,
    matches: u32,
    /// Quick Look counts the messages whose own words or files matched, and the subject or sender
    /// once for all the rest (#2085); `matches` counts every matched message, for the order.
    shown: u32,
    by_metadata: bool,
    unread: bool,
    flagged: bool,
    answered: bool,
    attachment: bool,
    action: bool,
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
            shown: 0,
            by_metadata: false,
            unread: false,
            flagged: false,
            answered: false,
            attachment: false,
            action: false,
            senders: Vec::new(),
            recipients: Vec::new(),
            labels: Vec::new(),
            folders: Vec::new(),
        }
    }

    fn add(&mut self, found: &[Found], at: usize) {
        let message = &found[at];
        let best = &found[self.best];
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
        if message.in_body || message.in_file {
            self.shown += 1;
        } else {
            self.by_metadata = true;
        }
        self.unread |= message.unread;
        self.flagged |= message.flagged;
        self.answered |= message.answered;
        self.attachment |= message.attachment;
        self.action |= message.action;
        self.senders.extend(&message.senders);
        self.recipients.extend(&message.recipients);
        self.labels.extend(&message.labels);
        self.folders.push(message.mailbox);
    }
}

const FACET_PEOPLE: usize = 50;

/// A conversation counts once for a value when any matched message has it, which is when the query
/// with that term added would still find it. The months use the newest match.
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

fn people_in(conversations: &[Conversation], own: &[i64]) -> u64 {
    let mut everyone: Vec<i64> = conversations
        .iter()
        .flat_map(|conversation| conversation.senders.iter().chain(&conversation.recipients))
        .copied()
        .filter(|id| !own.contains(id))
        .collect();
    everyone.sort_unstable();
    everyone.dedup();
    everyone.len() as u64
}

struct Order {
    ranked: Vec<usize>,
    pool: Vec<usize>,
}

impl Order {
    fn len(&self) -> usize {
        self.ranked.len()
    }

    fn rank(
        self,
        fold: &Fold,
        conversations: &[Conversation],
        exact: &[bool],
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
            exact[*b_at]
                .cmp(&exact[*a_at])
                .then_with(|| a.order(b, more))
        });
        let mut order: Vec<usize> = keyed.into_iter().map(|(_, at)| at).collect();
        order.extend(self.ranked.into_iter().skip(self.pool.len()));
        order
    }
}

fn order(
    fold: &Fold,
    conversations: &[Conversation],
    exact: &[bool],
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
                exact[*b]
                    .cmp(&exact[*a])
                    .then(keys[*a].total_cmp(&keys[*b]))
                    .then(conversations[*b].newest.cmp(&conversations[*a].newest))
                    .then(best(*b).id.cmp(&best(*a).id))
            });
            let pool = ranked[..ranked.len().min(CONVERSATION_POOL)].to_vec();
            Order { ranked, pool }
        }
    }
}

fn pool_key(best: &Found, now: DateTime<Utc>) -> f64 {
    let age = (now.timestamp_millis() - best.aged_from) as f64;
    best.text + POOL_AGE_WEIGHT_PER_YEAR * age / MILLIS_PER_YEAR
}

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

#[derive(Debug, Clone, Default)]
struct Hydrated {
    subject: Option<String>,
    from_name: Option<String>,
    from_address: Option<String>,
    times_seen: i64,
    sent_count: i64,
    thread: Option<(u32, bool, bool)>,
    number: Option<u32>,
    files: Vec<(AttachmentId, String)>,
}

/// Ordered and filtered as the thread's own read does (`ThreadRepository::members`). The outer side
/// of the thread match is an expression
/// so the engine keeps this a seek per row (docs/gotchas.md, "group-first").
const NUMBER: &str = "(SELECT count(*) FROM messages x
                        WHERE x.thread_id = m.thread_id + 0
                          AND x.received_at <= m.received_at
                          AND (x.received_at, x.id) <= (m.received_at, m.id)
                          AND x.deleted_locally = 0
                          AND (x.snoozed_until IS NULL
                               OR x.snoozed_until <= (strftime('%s','now') * 1000))
                          AND x.draft = 0)";

const UNIT: char = '\u{1f}';

/// The sender's `recipients` row is found once and joined by primary key, and the contacts probe
/// compares against a plain column, as [`Plan::hydrate_sql`](super::Plan) does.
async fn hydrate(
    connection: &Connection,
    account: AccountScope,
    ids: &[i64],
) -> Result<HashMap<i64, Hydrated>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let affinity = match account.account() {
        Some(_) => {
            "(SELECT s.times_seen FROM contact_sightings s
               WHERE s.address_id = a.id AND s.account_id = ?)"
        }
        None => "(SELECT c.times_seen FROM contacts c WHERE c.id = a.contact_id)",
    };
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT sub.id, sub.subject, sender.name, a.address,
                {affinity},
                (SELECT w.sent_count FROM correspondents w WHERE w.address_id = a.id),
                sub.thread_messages, sub.thread_unread, sub.thread_files, sub.files,
                sub.number
           FROM (SELECT m.id, m.subject,
                        (SELECT r.id FROM recipients r
                          WHERE r.message_id = m.id AND r.kind = 'from'
                          ORDER BY r.position LIMIT 1) AS from_recipient,
                        t.message_count AS thread_messages,
                        t.unread_count AS thread_unread,
                        t.has_attachments AS thread_files,
                        {NUMBER} AS number,
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
                number: row
                    .col::<Option<i64>>(10)?
                    .map(|number| u32::try_from(number.max(1)).unwrap_or(u32::MAX)),
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

struct Terms {
    subject: Vec<String>,
    files: Vec<String>,
    /// The words alone: a `from:` filter names a person, not words in a name.
    sender: Vec<String>,
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
                    .flat_map(|clause| clause.filter.alternatives())
                    .filter_map(|filter| field(filter).cloned()),
            );
            terms
        };
        Self {
            sender: words.clone(),
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

    // The body is known from which index matched it; a subject or file name from whether the words
    // are in it, as the row will highlight them, so the reason and the highlight cannot disagree.
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
    let in_sender = [row.from_name.as_deref(), row.from_address.as_deref()]
        .into_iter()
        .flatten()
        .any(|said| !postio_search::highlight::find(said, &terms.sender).is_empty());
    if in_sender {
        reasons.push(RankReason::InSender);
    }
    // A file found it by its name or else by what it says: a match only inside the file does not
    // say the name (#2194).
    if !named.is_empty() {
        reasons.push(RankReason::InFileName);
    } else if !read.is_empty() {
        reasons.push(RankReason::InFileContent);
    }
    reasons.push(RankReason::Matches(
        conversation.shown + u32::from(conversation.by_metadata),
    ));

    let (messages, unread, has_attachments) = match (conversation.key, row.thread) {
        (ConversationKey::Thread(_), Some(thread)) => thread,
        _ => (1, best.unread, best.attachment),
    };
    ConversationHit {
        key: conversation.key,
        best: MessageId::new(best.id),
        best_number: match conversation.key {
            ConversationKey::Thread(_) => row.number.unwrap_or(1),
            ConversationKey::Lone(_) => 1,
        },
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
        matched_messages: conversation.shown,
        matches,
    }
}

struct Searched {
    messages: u64,
    bodies: bool,
    contents: bool,
    own: Vec<i64>,
}

/// Read off the schema-kept counts of the folders in scope (`total_count`, `bodies_owed`), with
/// attachments answered beside it from the set the index keeps
/// (`attachment_text_owed`): one seek, where asking the queue walked every attachment on the
/// machine. Attachments are every account's, as the indexer's queue is:
/// the search cannot say it looked inside files while any file on this machine is unread.
async fn searched(connection: &Connection, request: &ConversationRequest<'_>) -> Result<Searched> {
    let mut conditions = Vec::new();
    let mut params: Vec<turso::Value> = Vec::new();
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
    let (total, owed, unread, own): (i64, i64, bool, Option<String>) = sql::one(
        connection,
        &format!(
            "SELECT coalesce(sum(total_count), 0), coalesce(max({owed} > 0), 0),
                    EXISTS (SELECT 1 FROM attachment_text_owed),
                    (SELECT group_concat(id) FROM addresses
                      WHERE address_normalized IN ({own}))
               FROM mailboxes WHERE {}",
            conditions.join(" AND "),
            owed = postio_storage::schema::BODIES_STILL_OWED,
            own = super::OWN_ADDRESSES,
        ),
        params,
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?, row.opt_text(3)?)),
    )
    .await?;
    Ok(Searched {
        messages: total.max(0) as u64,
        bodies: owed == 0,
        contents: !unread,
        own: own
            .unwrap_or_default()
            .split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect(),
    })
}
