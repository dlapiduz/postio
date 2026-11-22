//! Query execution: structured filters combined with the full-text index, ranked, with snippets.
//! [`search`] fetches a bounded candidate pool by `bm25` (or by recency past
//! [`RANK_BY_RELEVANCE_LIMIT`] matches) and re-ranks it in Rust with the pure [`rank_score`],
//! keeping scoring out of SQL.

use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use postio_model::{AccountScope, EmailAddress, MailboxId, MessageId, ThreadId};

use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};
use postio_search::query::{Filter, ParsedQuery, fts_literal};
use postio_search::results::{SearchHit, SearchResults, TOTAL_HITS_CAP};

use crate::error::Result;
use postio_storage::Connection;
use postio_storage::sql::{self, RowExt as _};

mod completions;
mod conversations;
mod files;
mod people;
mod relaxations;
pub use completions::{COMPLETION_COUNT_CAP, completions};
pub use conversations::{ConversationRequest, search_conversations};
pub use files::{FILES_CAP, FileMatch, file_matches, files};
pub use people::{OWN_ADDRESSES, PEOPLE_CAP, people};
pub use relaxations::relaxation_counts;

const CANDIDATE_POOL_MULTIPLIER: u32 = 5;

const CANDIDATE_POOL_MIN: u32 = 200;

/// Past this, ranking every match is not affordably fast (see the comment in [`search`]).
const RANK_BY_RELEVANCE_LIMIT: u64 = 2_000;

/// Ordering and join form are separate decisions: past [`RANK_BY_RELEVANCE_LIMIT`] a match orders
/// by recency, but which side drives the join is set here.
/// The probed shape's cost is set by how recent the matches are, not how many, so it is slow for a
/// word scattered through old mail (`invoice`, 3,843 hits: 972 ms probed, 142 ms driven)
/// and fast for one in most of the mailbox (`the`: 74 ms against 1.26 s). The two cross near 8,000.
/// The count saturates at `TOTAL_HITS_CAP`.
const PROBED_FORM_LIMIT: u64 = 8_000;

const RECENCY_POOL_MULTIPLIER: u32 = 2;
const RECENCY_POOL_MIN: u32 = 50;

/// Raised with the half-life below; a heavier weight on a term that is zero for every candidate
/// changes nothing. Calibrated against `bm25()`, then re-measured against `fts_score`, whose spread
/// is about a quarter of bm25's (0.20 over the whole match set) and often tied at the top, so the
/// blend is more recency-led than it was. Unchanged as a decision: #1216 complained of very old
/// mail surfacing. A clearly better match must still beat a small recency difference
/// (`newest_order_answers_in_date_order_however_the_ranking_disagrees`, `ranking_weights`).
const RECENCY_WEIGHT: f64 = 3.0;
/// Two years, not fourteen days: on a real store the forty best matches for `invoice` all had a
/// median age of 6,687 days, where a fourteen-day half-life is zero to four decimals,
/// so recency moved nothing and `bm25` alone surfaced very old mail. What matters is that recency
/// still differs between candidates; one year cannot reorder against a 0.80 bm25 spread,
/// two years can, and still separates last week from last month, which a five-year half-life starts
/// to flatten.
const RECENCY_HALF_LIFE_DAYS: f64 = 730.0;

/// Linear because the store engine has no `exp`, `ln` or `pow`. It decides which candidates enter
/// the pool, which `rank_score` only reorders, so raising `RECENCY_WEIGHT` alone would not surface
/// recent mail.
const POOL_AGE_WEIGHT_PER_YEAR: f64 = 0.25;

/// The text score separates near-identical mail by noise (three invoices from one template scored
/// within a few percent), so within this tolerance matches are one band and recency and the sender
/// decide.
const TEXT_TIE: f64 = 0.10;

/// Wider for messages whose subject and sender say every word: they read as a dated series, newest
/// first.
const SAID_TIE: f64 = 0.50;

/// A band is measured from its best, never its last member, so small steps cannot drift a weak
/// match into a strong one's band.
pub fn text_bands(bm25: &[f64]) -> Vec<f64> {
    bands(bm25, TEXT_TIE)
}

fn bands(bm25: &[f64], tie: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..bm25.len()).collect();
    order.sort_by(|a, b| bm25[*a].total_cmp(&bm25[*b]));
    let mut banded = bm25.to_vec();
    let mut best: Option<f64> = None;
    for index in order {
        let score = bm25[index];
        let lead = match best {
            Some(lead) if lead < 0.0 && score <= lead * (1.0 - tie) => lead,
            _ => score,
        };
        best = Some(lead);
        banded[index] = lead;
    }
    banded
}

/// A prefix, so "invoice" covers "invoices". The free-text score credits subject and sender only
/// when every term is there, so this separates a message about something from one that mentions it.
pub fn coverage(terms: &[String], said: &str) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let words: Vec<String> = said
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let covered = terms
        .iter()
        .filter(|term| {
            term.split_whitespace().all(|part| {
                let part = part.to_lowercase();
                words.iter().any(|word| word.starts_with(&part))
            })
        })
        .count();
    covered as f64 / terms.len() as f64
}

/// Coarse on purpose, so a sender's two addresses seen four and five times are one step.
fn known(times_seen: i64) -> u8 {
    let affinity = (1.0 + times_seen.max(0) as f64).ln() / (1.0 + 100f64).ln();
    (affinity.min(1.0) * 4.0).floor().min(3.0) as u8
}

#[derive(Debug, Clone, Copy)]
struct RelevanceKey {
    band: f64,
    said: f64,
    known: u8,
    aged_from: DateTime<Utc>,
    score: f64,
}

impl RelevanceKey {
    /// `between` decides what the subject and sender leave tied: [`search`] passes `Equal`, a
    /// conversation search the match count.
    fn order(&self, other: &Self, between: std::cmp::Ordering) -> std::cmp::Ordering {
        // In a band: what the subject and sender say, then how well the sender is known in coarse
        // steps, then the newer.
        self.band
            .total_cmp(&other.band)
            .then(other.said.total_cmp(&self.said))
            .then(between)
            .then(other.known.cmp(&self.known))
            .then(other.aged_from.cmp(&self.aged_from))
            .then(self.score.total_cmp(&other.score))
    }
}

fn relevance_keys(
    candidates: &mut [Candidate],
    query: &ParsedQuery,
    now: DateTime<Utc>,
) -> Vec<RelevanceKey> {
    let texts = text_bands(
        &candidates
            .iter()
            .map(|candidate| candidate.bm25)
            .collect::<Vec<_>>(),
    );
    for (candidate, text) in candidates.iter_mut().zip(texts) {
        candidate.score = rank_score(text, candidate.aged_from, now, candidate.sender_times_seen);
    }
    // Recency and affinity are too weak to order recent mail on their own, so within a band the
    // message whose subject and sender say more of the query leads; otherwise a sender's own
    // newsletters came back in no visible order.
    let terms: Vec<String> = query
        .searchable_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.clone())
        .collect();
    let said: Vec<f64> = candidates
        .iter()
        .map(|candidate| {
            let said = [
                candidate.subject.as_deref(),
                candidate.from_name.as_deref(),
                candidate.from_address.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            coverage(&terms, &said)
        })
        .collect();
    let mut banded = vec![0.0; candidates.len()];
    for all in [true, false] {
        let members: Vec<usize> = (0..candidates.len())
            .filter(|index| (said[*index] >= 1.0) == all)
            .collect();
        let scores: Vec<f64> = members
            .iter()
            .map(|index| candidates[*index].score)
            .collect();
        let tie = if all { SAID_TIE } else { TEXT_TIE };
        for (index, band) in members.into_iter().zip(bands(&scores, tie)) {
            banded[index] = band;
        }
    }
    candidates
        .iter()
        .zip(banded)
        .zip(said)
        .map(|((candidate, band), said)| RelevanceKey {
            band,
            said,
            known: known(candidate.sender_times_seen),
            aged_from: candidate.aged_from,
            score: candidate.score,
        })
        .collect()
}

/// `received_at` is when it arrived in its folder, so a move or resync re-dates it and last month's
/// invoice archived this morning outranked this week's mail. The `Date` header is taken only when
/// earlier: a future date never lifts a message.
const AGED_FROM: &str = "CASE WHEN m.date IS NOT NULL AND m.date < m.received_at \
     THEN m.date ELSE m.received_at END";

const MILLIS_PER_YEAR: f64 = 31_557_600_000.0;
/// Bounded in `[0, 1)` before weighting; at worst it orders a tie by who writes most.
const SENDER_WEIGHT: f64 = 1.0;

#[derive(Debug, Clone, Copy)]
pub struct SearchRequest<'a> {
    /// Orthogonal to [`scope`](Self::scope), the role tri-tab, so "this account's inbox" and "every
    /// account's inbox" are both askable (ADR 0005 Q5).
    pub account: AccountScope,
    pub query: &'a ParsedQuery,
    /// A constraint the query text never carries: switching scope must not mean editing what was
    /// typed.
    pub scope: Scope,
    pub limit: u32,
    /// `Newest` takes the recency-ordered fetch and skips the re-rank, so the answer is exactly a
    /// mailbox's own order (#499).
    pub order: postio_search::ResultOrder,
}

/// `now` is the reference clock for the recency boost, a parameter so ranking is a pure function of
/// its inputs.
pub async fn search(
    connection: &Connection,
    request: &SearchRequest<'_>,
    now: DateTime<Utc>,
) -> Result<SearchResults> {
    let exact = search_as(connection, request, now, &Default::default()).await?;
    // The forgiving search the command bar asks for (ADR 0037): exact matches first, then, only if
    // they do not fill the page, the words near them. A rule's query is never forgiving.
    if !request.query.is_forgiving() || exact.hits.len() >= request.limit as usize {
        return Ok(exact);
    }
    let start = Instant::now();
    let words = near_words(connection, request).await?;
    if words.is_empty() {
        return Ok(exact);
    }
    let near = search_as(connection, request, now, &words).await?;
    let mut merged = exact;
    let have: std::collections::HashSet<MessageId> =
        merged.hits.iter().map(|hit| hit.message_id).collect();
    let room = (request.limit as usize).saturating_sub(merged.hits.len());
    merged.hits.extend(
        near.hits
            .into_iter()
            .filter(|hit| !have.contains(&hit.message_id))
            .take(room),
    );
    merged.total_hits = merged.total_hits.max(near.total_hits);
    merged.total_hits_capped |= near.total_hits_capped;
    merged.elapsed += start.elapsed();
    Ok(merged)
}

async fn search_as(
    connection: &Connection,
    request: &SearchRequest<'_>,
    now: DateTime<Utc>,
    near: &std::collections::HashMap<String, Vec<String>>,
) -> Result<SearchResults> {
    let start = Instant::now();
    let plan = Plan::build_near(request, near);

    let total_hits = plan.count(connection).await?;
    let total_hits_capped = total_hits >= TOTAL_HITS_CAP;
    // A term matched by most of a large mailbox has no cheap top-K by `bm25` (a full sort cost
    // several hundred ms, over the `<100 ms` budget), so past `RANK_BY_RELEVANCE_LIMIT` matches
    // `fetch` orders by recency, which the list index answers with no sort.
    let rank_by_relevance = request.order == postio_search::ResultOrder::Relevance
        && plan.has_match
        && total_hits <= RANK_BY_RELEVANCE_LIMIT;
    // The wider pool only pays off when ranking by relevance, so a worse `bm25` match with better
    // recency can surface;
    // the recency path is the broad-match one, where hydrating a smaller pool matters.
    let pool_size = if rank_by_relevance {
        request
            .limit
            .saturating_mul(CANDIDATE_POOL_MULTIPLIER)
            .max(CANDIDATE_POOL_MIN)
    } else {
        request
            .limit
            .saturating_mul(RECENCY_POOL_MULTIPLIER)
            .max(RECENCY_POOL_MIN)
    };
    let mut candidates = plan
        .fetch(connection, pool_size, rank_by_relevance, total_hits, now)
        .await?;

    match request.order {
        postio_search::ResultOrder::Relevance => {
            let keys = relevance_keys(&mut candidates, request.query, now);
            let mut keyed: Vec<(RelevanceKey, Candidate)> =
                keys.into_iter().zip(candidates).collect();
            keyed.sort_by(|(a, _), (b, _)| a.order(b, std::cmp::Ordering::Equal));
            candidates = keyed.into_iter().map(|(_, candidate)| candidate).collect();
            for candidate in candidates.iter().take(request.limit as usize) {
                tracing::trace!(
                    id = candidate.message_id.get(),
                    text = candidate.bm25,
                    age_days = (now - candidate.aged_from).num_hours() as f64 / 24.0,
                    sender_seen = candidate.sender_times_seen,
                    score = candidate.score,
                    "ranked"
                );
            }
        }
        // Newest order: the fetch already came back `received_at DESC`, and ranking would let
        // affinity reorder what the control promises is a plain sort.
        postio_search::ResultOrder::Newest => {}
    }
    candidates.truncate(request.limit as usize);

    let hits: Vec<_> = candidates.into_iter().map(Candidate::into_hit).collect();
    let elapsed = start.elapsed();

    // Shape and cost, never the query: what someone searches their own mail for is as revealing as
    // the mail.
    tracing::debug!(
        terms = request.query.text_terms().count(),
        filters = request.query.filters().count(),
        hits = hits.len(),
        total_hits,
        capped = total_hits_capped,
        by_relevance = rank_by_relevance,
        elapsed_ms = elapsed.as_millis(),
        "search finished"
    );

    Ok(SearchResults {
        // Only when nothing matched: the one moment the cost is free.
        suggestion: match total_hits {
            0 => suggestion_for(connection, request.query).await?,
            _ => None,
        },
        hits,
        total_hits,
        total_hits_capped,
        elapsed,
        corpus_complete: corpus_complete(connection, request).await?,
        instead: None,
    })
}

/// Only for a single bare word: with two terms correcting one is a guess, and with a filter the
/// filter is the likelier reason nothing matched.
/// Candidates come from the index's term dictionary via [`postio_search::suggest::widened`], and
/// the word is recovered from the text of at most
/// [`SUGGESTION_DOCUMENTS`] documents of each half, so the count shown is "at least". Also read by
/// [`near_words`], never for a rule's query (ADR 0037).
async fn suggestion_for(
    connection: &Connection,
    query: &postio_search::ParsedQuery,
) -> Result<Option<postio_search::suggest::Suggestion>> {
    let mut terms = query.searchable_terms();
    let Some(term) = terms.next() else {
        return Ok(None);
    };
    if terms.next().is_some() || term.negated || term.quoted || query.filters().next().is_some() {
        return Ok(None);
    }
    let counts = words_near(connection, &term.value, &[]).await?;
    Ok(postio_search::suggest::suggest(
        &term.value,
        counts
            .iter()
            .map(|(text, documents)| postio_search::suggest::Term {
                text,
                documents: *documents,
            }),
    ))
}

/// Empty when `typed` cannot be widened at all. Recovered from the text of at most
/// [`SUGGESTION_DOCUMENTS`] documents of each half, since the index answers with rows, not the
/// terms it expanded to.
async fn words_near(
    connection: &Connection,
    typed: &str,
    beside: &[&str],
) -> Result<std::collections::HashMap<String, u64>> {
    use std::collections::{HashMap, HashSet};

    let Some(widened) = postio_search::suggest::widened(typed) else {
        return Ok(HashMap::new());
    };
    // The words typed beside it, bare, so the index reads one query of bare words (the only kind it
    // expands `~N` in) and the documents read best match all of it; without them "trop" never
    // reached the one scout troop.
    let beside: Vec<String> = beside
        .iter()
        .map(|word| word.to_lowercase())
        .filter(|word| !word.is_empty() && word.chars().all(char::is_alphanumeric))
        .collect();
    let metadata_query = std::iter::once(widened.clone())
        .chain(beside.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    // The body column is folded on the way in, so its query is folded too (ADR 0038).
    let body_query =
        postio_search::suggest::widened(&postio_model::fold::fold(typed)).map(|widened| {
            std::iter::once(widened)
                .chain(beside.iter().map(|word| postio_model::fold::fold(word)))
                .collect::<Vec<_>>()
                .join(" ")
        });

    // Best first, so a common neighbour cannot crowd out the meant word. `fts_score` is projected
    // bare with the match's own parameter, or it answers 0.0 (see `HITS_JOIN`).
    let mut texts: Vec<Vec<Option<String>>> = sql::all(
        connection,
        "SELECT sender, recipients, subject, filenames, list_id,
                fts_score(sender, recipients, subject, filenames, list_id, ?1) AS score
           FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
          ORDER BY score DESC
          LIMIT ?2",
        (metadata_query, SUGGESTION_DOCUMENTS),
        |row| {
            Ok(vec![
                row.opt_text(0)?,
                row.opt_text(1)?,
                row.opt_text(2)?,
                row.opt_text(3)?,
                row.opt_text(4)?,
            ])
        },
    )
    .await?;
    if let Some(body_query) = body_query {
        texts.extend(
            sql::all(
                connection,
                "SELECT body_search, fts_score(body_search, ?1) AS score
                   FROM message_search_bodies
                  WHERE fts_match(body_search, ?1)
                  ORDER BY score DESC
                  LIMIT ?2",
                (body_query, SUGGESTION_DOCUMENTS),
                |row| Ok(vec![row.opt_text(0)?]),
            )
            .await?,
        );
    }

    let mut counts: HashMap<String, u64> = HashMap::new();
    for document in &texts {
        let mut seen: HashSet<String> = HashSet::new();
        for text in document.iter().flatten() {
            for word in text
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
            {
                let word = word.to_lowercase();
                if !seen.contains(&word) {
                    *counts.entry(word.clone()).or_default() += 1;
                    seen.insert(word);
                }
            }
        }
    }

    Ok(counts)
}

/// A word with nothing near it beyond itself, or that cannot be widened, is left out and searched
/// as typed.
async fn near_words(
    connection: &Connection,
    request: &SearchRequest<'_>,
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let mut near = std::collections::HashMap::new();
    let words: Vec<&str> = request
        .query
        .searchable_terms()
        .filter(|term| !term.negated)
        .map(|term| term.value.as_str())
        .collect();
    for term in request.query.searchable_terms() {
        if term.negated || term.quoted || near.contains_key(&term.value) {
            continue;
        }
        let beside: Vec<&str> = words
            .iter()
            .copied()
            .filter(|word| *word != term.value)
            .collect();
        let mut counts = words_near(connection, &term.value, &beside).await?;
        // Whether the mailbox holds the word decides whether it is corrected, so it is asked, not
        // inferred from the documents read.
        let typed = term.value.to_lowercase();
        if !counts.contains_key(&typed) && holds(connection, &typed).await? {
            counts.insert(typed, 1);
        }
        let words = postio_search::suggest::near(
            &term.value,
            counts
                .iter()
                .map(|(text, documents)| postio_search::suggest::Term {
                    text,
                    documents: *documents,
                }),
            NEAR_WORDS,
        );
        if words.iter().any(|word| *word != term.value.to_lowercase()) {
            near.insert(term.value.clone(), words);
        }
    }
    Ok(near)
}

async fn holds(connection: &Connection, word: &str) -> Result<bool> {
    let literal = fts_literal(word);
    Ok(sql::exists(
        connection,
        "SELECT 1 FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)",
        (literal.clone(),),
    )
    .await?
        || sql::exists(
            connection,
            "SELECT 1 FROM message_search_bodies WHERE fts_match(body_search, ?1)",
            (postio_model::fold::fold(&literal),),
        )
        .await?)
}

const NEAR_WORDS: usize = 8;

/// A boolean, not a count: under ADR 0016 the missing number is a draining queue that needs no
/// action (#352).
/// Read off the folders (#1612): each mailbox keeps `bodies_owed` and the scope is always a set of
/// mailboxes, so one short walk fits the `<100 ms` budget. Scoped to the searched mailboxes.
/// It asks whether backfill still owes bodies, not whether every local body is indexed: that needs
/// a full-corpus scan per keystroke. Under-reporting for a few seconds after launch is the right
/// way to be wrong.
async fn corpus_complete(connection: &Connection, request: &SearchRequest<'_>) -> Result<bool> {
    let (sql, params) = corpus_complete_sql(request);
    let complete = sql::one(connection, &sql, params, |row| row.col(0)).await?;
    Ok(complete)
}

/// Read off the folders, not the messages (#1612); exposed so a test can ask the planner what it
/// reads.
#[doc(hidden)]
pub fn corpus_complete_sql(request: &SearchRequest<'_>) -> (String, Vec<turso::Value>) {
    let mut conditions = vec![format!("{} > 0", postio_storage::schema::BODIES_STILL_OWED)];
    let mut params: Vec<turso::Value> = Vec::new();
    if let Some(id) = request.account.account() {
        conditions.push("account_id = ?".to_string());
        params.push(turso::Value::Integer(id.get()));
    }
    if let Some(role) = scope_role(request.scope, names_a_folder(request.query)) {
        conditions.push(role.to_string());
    }
    (
        format!(
            "SELECT NOT EXISTS (SELECT 1 FROM mailboxes WHERE {})",
            conditions.join(" AND ")
        ),
        params,
    )
}

/// One threshold, not a ramp: a second size chip would be two ways of saying the same thing.
const LARGE_BYTES: u64 = 1024 * 1024;

const LARGE_TOKEN: &str = "larger:1M";

/// Two, so a mailbox that files list traffic into a dozen folders does not crowd out `is:unread`.
const REFINE_FOLDERS: usize = 2;

/// Bounds only how common a candidate can be shown to be: ranking needs relative counts. Runs only
/// on a search that found nothing.
const SUGGESTION_DOCUMENTS: i64 = 50;

/// Separate from [`search`] because the facets only need to be right by the time the eye reaches
/// the column; making every keystroke pay for four aggregate queries would spend the interaction
/// budget. Every count is capped like [`SearchResults::total_hits`] ([`TOTAL_HITS_CAP`]).
pub async fn facets(connection: &Connection, request: &SearchRequest<'_>) -> Result<Facets> {
    // Scope counts hold the query and vary the scope: the column says what switching would find.
    // Refinements are measured inside the current scope and share its capped walk (#1612); only the
    // other scopes walk the match again, since one shared walk could let a broad match in a large
    // folder fill the cap and read another scope as empty.
    let here = Plan::build(request).current_scope(connection).await?;
    let mut scopes = Vec::with_capacity(Scope::ALL.len());
    for scope in Scope::ALL {
        let hits = if scope == request.scope {
            here.hits
        } else {
            Plan::build(&SearchRequest { scope, ..*request })
                .count(connection)
                .await?
        };
        scopes.push(ScopeCount { scope, hits });
    }

    Ok(Facets {
        scopes,
        refinements: here.refinements,
    })
}

struct CurrentScope {
    hits: u64,
    refinements: Vec<Refinement>,
}

/// Both boosts are bounded in `[0, 1)` before weighting, so a much stronger text match is never
/// overridden: they break ties and nudge close calls.
pub fn rank_score(
    bm25: f64,
    received_at: DateTime<Utc>,
    now: DateTime<Utc>,
    sender_times_seen: i64,
) -> f64 {
    let age_days = (now - received_at).num_milliseconds() as f64 / 86_400_000.0;
    let age_days = age_days.max(0.0);
    let recency = (-age_days / RECENCY_HALF_LIFE_DAYS * std::f64::consts::LN_2).exp();

    // log1p rather than the raw count: once versus five times should matter more than 500 versus
    // 504.
    let affinity = (1.0 + sender_times_seen.max(0) as f64).ln() / (1.0 + 100f64).ln();
    let affinity = affinity.min(1.0);

    bm25 - RECENCY_WEIGHT * recency - SENDER_WEIGHT * affinity
}

/// Body at half, summed rather than the better of the two, so a message matching in both ranks
/// first. The two scores share no scale, so half encodes only the ordering "subject beats body,
/// both beats either" (`a_subject_match_outranks_a_body_match`). bm25 is negative-is-better, so a
/// missing side contributes zero.
const BODY_SCORE_WEIGHT: f64 = 0.5;

/// `UNION ALL` of two `MATCH`es, not an `OR`: FTS5 needs `MATCH` as an index constraint. No `GROUP
/// BY`: an aggregate materialises every match first (a 120,000-row temp table, 297 ms against 100
/// ms), so a message matching in both appears twice; callers dedupe (the pool in Rust, counts with
/// `DISTINCT`, `hydrate` aggregates). The matches are written first and pinned there, or SQLite
/// drives from `messages` (49 ms against 2.9). `fts_score` is projected bare and negated outside:
/// inside any arithmetic it answers `0.0`, silently dropping the ranking
/// (`the_score_is_lost_to_any_arithmetic_around_it` in `postio-storage` will notice a fix). It also
/// answers `0.0` unless its query term is the same parameter as the `fts_match` that selected the
/// row, so the term is `?1`/`?2` once per index and reused (`match_params` binds two values). The
/// lookup names `idx_messages_content`: the planner has no statistics and ranks a seek by key
/// columns bound, so `m.received_at >= ?` offered `idx_messages_account_list` and walked every
/// message in range per hit (#1809); `driven_join_plan` holds it.
const HITS_JOIN: &str = "FROM (
             SELECT content_id AS rid,
                    fts_score(sender, recipients, subject, filenames, list_id, ?1) AS meta,
                    NULL AS body
               FROM search_documents
              WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
             UNION ALL
             SELECT content_id, NULL, fts_score(body_search, ?2)
               FROM message_search_bodies
              WHERE fts_match(body_search, ?2)
          ) hits CROSS JOIN messages m INDEXED BY idx_messages_content
                 ON m.content_id = hits.rid";

/// The conversation search's own: keep the first two arms and the join in step with [`HITS_JOIN`].
/// The attachment arm is grouped by content, so a spreadsheet whose every row says the word is one
/// hit; its text is folded like the body and matched by `?2`. Every statement looking messages up
/// by content names `idx_messages_content` (#1809), held by `driven_join_plan`.
const HITS_JOIN_WITH_FILES: &str = "FROM (
             SELECT content_id AS rid,
                    fts_score(sender, recipients, subject, filenames, list_id, ?1) AS meta,
                    NULL AS body, NULL AS file
               FROM search_documents
              WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
             UNION ALL
             SELECT content_id, NULL, fts_score(body_search, ?2), NULL
               FROM message_search_bodies
              WHERE fts_match(body_search, ?2)
             UNION ALL
             SELECT content_id, NULL, NULL, max(unit_score)
               FROM (SELECT content_id, fts_score(text_search, ?2) AS unit_score
                       FROM attachment_passages
                      WHERE fts_match(text_search, ?2))
              GROUP BY content_id
          ) hits CROSS JOIN messages m INDEXED BY idx_messages_content
                 ON m.content_id = hits.rid";

/// Driven by `messages` rather than by the match, for a query too broad to rank: ordered by
/// `m.received_at` off its own index. The ask must be a docid seek
/// (`rowid = m.id AND ... MATCH ?`); probing [`HITS_JOIN`]'s union per row, or `m.id IN (SELECT
/// rowid ...)`, builds a temp table, about 120 ms on a word in most of the mailbox.
/// `meta` and `body` number the two terms: `?1` and `?2` driven, after the conditions probed.
fn correlated_match(alias: &str, meta: usize, body: usize) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM search_documents {alias}_d
               WHERE {alias}_d.content_id = {alias}.content_id
                 AND fts_match({alias}_d.sender, {alias}_d.recipients, {alias}_d.subject,
                               {alias}_d.filenames, {alias}_d.list_id, ?{meta}))
   OR EXISTS (SELECT 1 FROM message_search_bodies {alias}_b
               WHERE {alias}_b.content_id = {alias}.content_id
                 AND fts_match({alias}_b.body_search, ?{body})))"
    )
}

/// Exact is every fact the row keeps (account, `Message-ID`, size, `Date`, people): a `Message-ID`
/// alone never collapses, since a reused or forged one names another message (ADR 0046, #1526).
/// The seek names `idx_messages_rfc_message_id` as [`HITS_JOIN`] names its index. A content sibling
/// is `earlier`'s, left out so a Gmail row costs one seek.
fn copy_of(copy: &str) -> String {
    format!(
        "{copy}.account_id = m.account_id AND {copy}.rfc_message_id = m.rfc_message_id \
         AND {copy}.size = m.size AND m.size > 0 AND {copy}.date = m.date \
         AND {copy}.people_ids = m.people_ids AND {copy}.content_id <> m.content_id"
    )
}

/// Numbers first and the `Message-ID` last, so no two different rows spell the same key.
const COPY_KEY: &str = "CASE WHEN m.rfc_message_id IS NOT NULL AND m.size > 0
               AND m.date IS NOT NULL AND m.people_ids IS NOT NULL
          THEN m.account_id || ' ' || m.size || ' ' || m.date || ' '
               || m.people_ids || ' ' || m.rfc_message_id END";

/// Only the `m.` alias, not the suffix of another alias such as `gm.`.
fn realias(sql: &str, alias: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    for (offset, _) in sql.match_indices("m.") {
        if offset > 0
            && (sql.as_bytes()[offset - 1].is_ascii_alphanumeric()
                || sql.as_bytes()[offset - 1] == b'_')
        {
            continue;
        }
        out.push_str(&sql[cursor..offset]);
        out.push_str(alias);
        out.push('.');
        cursor = offset + 2;
    }
    out.push_str(&sql[cursor..]);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Driven,
    Probed,
}

#[derive(Clone)]
struct Candidate {
    message_id: MessageId,
    thread_id: Option<ThreadId>,
    mailbox_id: MailboxId,
    subject: Option<String>,
    from_name: Option<String>,
    from_address: Option<String>,
    received_at: DateTime<Utc>,
    aged_from: DateTime<Utc>,
    preview: Option<String>,
    snippet: String,
    bm25: f64,
    sender_times_seen: i64,
    score: f64,
}

impl Candidate {
    fn into_hit(self) -> SearchHit {
        SearchHit {
            message_id: self.message_id,
            thread_id: self.thread_id,
            mailbox_id: self.mailbox_id,
            subject: self.subject,
            from: self
                .from_address
                .map(|address| EmailAddress::new(self.from_name, address)),
            received_at: self.received_at,
            preview: self.preview,
            snippet: self.snippet,
            score: self.score,
        }
    }
}

struct Plan {
    conditions: Vec<String>,
    params: Vec<turso::Value>,
    /// Carried, not recovered from `params[0]`: under `AccountScope::Unified` there is no account
    /// parameter (#186).
    account: AccountScope,
    has_match: bool,
    /// A widened plan never takes the probed shape, which would run the disjunction once per
    /// message walked.
    widened: bool,
    /// Kept apart from `params`: a filter clause also binds its own `MATCH ?` and can come after
    /// it, and `hydrate` needs the free-text expression specifically.
    match_param: Option<turso::Value>,
    body_match_param: Option<turso::Value>,
    sets: Vec<IdSet>,
}

/// A conversation search's way round `m.id IN (subquery)`, which is tested once per row walked:
/// `budget from:ada` cost 375 ms on 20,000 messages, against about 1 ms read once as a set.
#[derive(Debug, Clone)]
struct IdSet {
    sql: String,
    params: Vec<turso::Value>,
    negated: bool,
}

/// The indexes are keyed by content, so each content is joined to every message carrying it, as
/// [`HITS_JOIN`] does.
const NEGATED_WORD_SET: &str = "SELECT m.id AS message_id FROM (
             SELECT content_id FROM search_documents
              WHERE fts_match(sender, recipients, subject, filenames, list_id, ?)
             UNION ALL
             SELECT content_id FROM message_search_bodies WHERE fts_match(body_search, ?)
          ) d CROSS JOIN messages m INDEXED BY idx_messages_content
                 ON m.content_id = d.content_id";

impl Plan {
    fn build(request: &SearchRequest<'_>) -> Self {
        Self::build_near(request, &Default::default())
    }

    fn build_sets(request: &SearchRequest<'_>) -> Self {
        Self::build_with(request, &Default::default(), true)
    }

    /// Every word is still a quoted literal, so the index reads the disjunction exactly as written.
    fn build_near(
        request: &SearchRequest<'_>,
        near: &std::collections::HashMap<String, Vec<String>>,
    ) -> Self {
        Self::build_with(request, near, false)
    }

    fn build_sets_near(
        request: &SearchRequest<'_>,
        near: &std::collections::HashMap<String, Vec<String>>,
    ) -> Self {
        Self::build_with(request, near, true)
    }

    fn build_with(
        request: &SearchRequest<'_>,
        near: &std::collections::HashMap<String, Vec<String>>,
        as_sets: bool,
    ) -> Self {
        // `Unified` names no account, so the predicate is absent: without `idx_messages_recency`
        // (migration 0012) the recency path has no index to supply its ordering (ADR 0005 Q5a).
        let mut conditions = vec!["m.deleted_locally = 0".to_string()];
        let mut params: Vec<turso::Value> = Vec::new();
        match request.account.account() {
            Some(id) => {
                conditions.push("m.account_id = ?".to_string());
                params.push(turso::Value::Integer(id.get()));
            }
            // `Unified` is every enabled account: dropping the account conjunct alone would reach a
            // switched-off or half-deleted account's mail (ADR 0005 Q10).
            // A subquery, not a join: `accounts` is a handful of rows, and a join would give the
            // planner a second table to order on the budgeted path.
            None => conditions.push(
                "m.account_id IN (SELECT id FROM accounts \
                  WHERE enabled = 1 AND pending_deletion = 0)"
                    .to_string(),
            ),
        }
        let account = request.account;
        let mut has_match = false;
        let mut match_param = None;
        let mut body_match_param = None;
        let mut sets = Vec::new();

        // Negated terms are excluded outside the join, across both indexes: negating inside the
        // metadata index alone returned a message whose excluded word was in its body.
        for term in request.query.text_terms().filter(|term| term.negated) {
            let literal = fts_literal(&term.value);
            if as_sets {
                sets.push(IdSet {
                    sql: NEGATED_WORD_SET.to_string(),
                    params: vec![
                        turso::Value::Text(literal.clone()),
                        turso::Value::Text(postio_model::fold::fold(&literal)),
                    ],
                    negated: true,
                });
                continue;
            }
            conditions.push(
                "m.content_id NOT IN (SELECT content_id FROM search_documents
                               WHERE fts_match(sender, recipients, subject,
                                               filenames, list_id, ?))
                 AND m.content_id NOT IN (SELECT content_id FROM message_search_bodies
                                   WHERE fts_match(body_search, ?))"
                    .to_string(),
            );
            params.push(turso::Value::Text(literal.clone()));
            params.push(turso::Value::Text(postio_model::fold::fold(&literal)));
        }

        let positive: Vec<Vec<&str>> = request
            .query
            .searchable_terms()
            .filter(|term| !term.negated)
            .map(
                |term| match near.get(&term.value).filter(|_| !term.quoted) {
                    Some(words) => words.iter().map(String::as_str).collect(),
                    None => vec![term.value.as_str()],
                },
            )
            .collect();
        // A disjunction of literals, not the index's `ticket~1`, which it expands only in a query
        // of bare words.
        // The body half folds each word, never the whole expression: folded, `OR` is the word "or".
        let expression = |folded: bool| {
            positive
                .iter()
                .map(|words| {
                    let literals: Vec<String> = words
                        .iter()
                        .map(|word| {
                            let literal = fts_literal(word);
                            if folded {
                                postio_model::fold::fold(&literal)
                            } else {
                                literal
                            }
                        })
                        .collect();
                    match literals.as_slice() {
                        [one] => one.clone(),
                        many => format!("({})", many.join(" OR ")),
                    }
                })
                .collect::<Vec<_>>()
                .join(" AND ")
        };
        if !positive.is_empty() {
            let expr = expression(false);
            body_match_param = Some(turso::Value::Text(expression(true)));
            // The match is in the join (`Plan::join_sql`): each index is matched in its own
            // subquery, since FTS5 needs `MATCH` as an index constraint, not under an `OR`.
            // Nothing is pushed onto `params`: the join's parameters precede every condition's, and
            // `from_params` binds them.
            match_param = Some(turso::Value::Text(expr));
            has_match = true;
        }

        if let Some((sql, mut values)) = scope_condition(
            request.scope,
            request.account,
            names_a_folder(request.query),
        ) {
            conditions.push(sql);
            params.append(&mut values);
        }

        let people = request.query.is_contact_aware();
        for clause in request.query.filters() {
            if as_sets && let Some((sql, params)) = id_set(&clause.filter, people) {
                sets.push(IdSet {
                    sql,
                    params,
                    negated: clause.negated,
                });
                continue;
            }
            let (sql, mut values) = filter_condition(&clause.filter, people);
            let sql = if clause.negated {
                format!("NOT ({sql})")
            } else {
                sql
            };
            conditions.push(sql);
            params.append(&mut values);
        }

        Self {
            conditions,
            params,
            account,
            has_match,
            widened: !near.is_empty(),
            match_param,
            body_match_param,
            sets,
        }
    }

    fn where_sql(&self, form: Form) -> String {
        let (eligible, next) = self.numbered(form);
        // A sibling must satisfy the same location predicates; reusing numbered parameters keeps
        // this a bounded indexed seek, including on the recency-driven path.
        let earlier = realias(&eligible, "earlier");
        // An exact copy (#1526) must match too: it has content of its own whose body may not be
        // here yet, and a non-matching copy hiding a matching one would lose the message.
        let (meta, body) = match form {
            Form::Driven => (1, 2),
            Form::Probed => (next, next + 1),
        };
        let copy = realias(&eligible, "copy");
        let copy_matched = if self.has_match {
            format!(" AND {}", correlated_match("copy", meta, body))
        } else {
            String::new()
        };
        let mut conditions = vec![
            eligible,
            format!(
                "NOT EXISTS (SELECT 1 FROM messages earlier INDEXED BY idx_messages_content \
             WHERE earlier.content_id = m.content_id AND earlier.id < m.id \
               AND ({earlier}))"
            ),
            format!(
                "NOT EXISTS (SELECT 1 FROM messages copy INDEXED BY idx_messages_rfc_message_id \
             WHERE {} AND copy.id < m.id AND ({copy}){copy_matched})",
                copy_of("copy")
            ),
        ];
        if self.has_match {
            conditions.push(match form {
                Form::Driven => "hits.rid IS NOT NULL".to_string(),
                Form::Probed => correlated_match("m", meta, body),
            });
        }
        conditions.join(" AND ")
    }

    fn occurrence_where_sql(&self, form: Form) -> String {
        self.numbered(form).0
    }

    fn numbered(&self, form: Form) -> (String, usize) {
        let mut parameter = if self.has_match && form == Form::Driven {
            3
        } else {
            1
        };
        let mut sql = String::new();
        for character in self.conditions.join(" AND ").chars() {
            if character == '?' {
                sql.push_str(&format!("?{parameter}"));
                parameter += 1;
            } else {
                sql.push(character);
            }
        }
        (sql, parameter)
    }

    /// Driven statements match in their `FROM`, so their two expressions come first; probed ones
    /// match in their `WHERE`, so they come last.
    fn params_for(&self, form: Form) -> Vec<turso::Value> {
        match form {
            Form::Driven => {
                let mut params = self.match_params(Form::Driven);
                params.extend(self.params.iter().cloned());
                params
            }
            Form::Probed => {
                let mut params = self.params.clone();
                params.extend(self.match_params(Form::Probed));
                params
            }
        }
    }

    fn source_sql(&self, form: Form) -> &'static str {
        if !self.has_match {
            return "FROM messages m";
        }
        match form {
            Form::Driven => HITS_JOIN,
            Form::Probed => "FROM messages m",
        }
    }

    /// They come before every condition's, because the join is written before the `WHERE` and
    /// SQLite binds `?` left to right.
    fn match_params(&self, _form: Form) -> Vec<turso::Value> {
        let (Some(expr), Some(folded)) = (&self.match_param, &self.body_match_param) else {
            return Vec::new();
        };
        // The body index is built over folded text, so its half is folded and the metadata half is
        // not (`postio_model::fold`). Each term is folded before joining:
        // folding the whole expression makes `AND` the word "and", and every body in a mailbox
        // matched (200 s to count).
        // Two values either way: the driven form uses each twice (score and match) since
        // `fts_score` returns 0.0 for a different parameter; the probed form's bare `?`s sit after
        // the conditions.
        vec![expr.clone(), folded.clone()]
    }

    /// `CROSS JOIN` pins the join order for a match too broad to rank: a plain `JOIN` had SQLite
    /// drive from `messages_fts` and sort the whole match set in a temp b-tree (0.75 s on
    /// postio-y47's "common word").
    fn fetch_form(&self, rank_by_relevance: bool, total_hits: u64) -> Form {
        if rank_by_relevance || self.widened || total_hits <= PROBED_FORM_LIMIT {
            Form::Driven
        } else {
            Form::Probed
        }
    }

    /// Counting a term in most of a mailbox exactly needs a full-postings walk, so the scan is
    /// wrapped in a `LIMIT` and is a floor past the cap ([`SearchResults::total_hits_capped`]).
    async fn count(&self, connection: &Connection) -> Result<u64> {
        let sql = format!(
            "SELECT count(*) FROM (SELECT DISTINCT m.id {} WHERE {} LIMIT ?)",
            self.source_sql(Form::Driven),
            self.where_sql(Form::Driven)
        );
        let mut params = self.params_for(Form::Driven);
        params.push(turso::Value::Integer(TOTAL_HITS_CAP as i64));
        let count: i64 = sql::one(connection, &sql, params.clone(), |row| row.col(0)).await?;
        Ok(count as u64)
    }

    /// One statement grouped by folder (#1612), where there were two walks and a third count of the
    /// same match. The inner `LIMIT` is [`TOTAL_HITS_CAP`], so every number is a floor past the
    /// cap.
    /// Folder chips are `in:`, not `list:`, which cannot yet be answered exactly (see
    /// [`Scope::Lists`], `postio-0bz`).
    /// `LARGE_BYTES` is written into the SQL, not bound: a bound `?` would sit before the `{from}`
    /// that [`HITS_JOIN`] numbers `?1`, and once bound five parameters into three slots.
    async fn current_scope(&self, connection: &Connection) -> Result<CurrentScope> {
        // Cap content identities first, then measure every qualifying membership: summing
        // per-folder counts would double-count content, and choosing one folder first would hide
        // folder refinements.
        let sql = format!(
            "WITH capped AS (
                 SELECT DISTINCT m.content_id {from} WHERE {where_sql} LIMIT ?
             ), matched AS (
                 SELECT m.content_id, mb.name, m.seen, m.flagged, m.has_attachments, m.size
                   FROM capped c CROSS JOIN messages m INDEXED BY idx_messages_content
                     ON m.content_id = c.content_id
                   JOIN mailboxes mb ON mb.id = m.mailbox_id
                  WHERE {eligible}
                 UNION ALL
                 SELECT m.content_id, mb.name, copy.seen, copy.flagged, copy.has_attachments,
                        copy.size
                   FROM capped c CROSS JOIN messages m INDEXED BY idx_messages_content
                     ON m.content_id = c.content_id
                   CROSS JOIN messages copy INDEXED BY idx_messages_rfc_message_id
                     ON {copy_of}
                   JOIN mailboxes mb ON mb.id = copy.mailbox_id
                  WHERE {eligible} AND {copy_eligible}{copy_matched}
             )
             SELECT NULL, count(DISTINCT content_id),
                    count(DISTINCT CASE WHEN seen = 0 THEN content_id END),
                    count(DISTINCT CASE WHEN flagged THEN content_id END),
                    count(DISTINCT CASE WHEN has_attachments THEN content_id END),
                    count(DISTINCT CASE WHEN size >= {LARGE_BYTES} THEN content_id END)
               FROM matched
             UNION ALL
             SELECT name, count(DISTINCT content_id), 0, 0, 0, 0
               FROM matched GROUP BY name",
            from = self.source_sql(Form::Driven),
            where_sql = self.where_sql(Form::Driven),
            eligible = self.occurrence_where_sql(Form::Driven),
            copy_of = copy_of("copy"),
            copy_eligible = realias(&self.occurrence_where_sql(Form::Driven), "copy"),
            copy_matched = if self.has_match {
                format!(" AND {}", correlated_match("copy", 1, 2))
            } else {
                String::new()
            },
        );
        let mut params = self.params_for(Form::Driven);
        params.push(turso::Value::Integer(TOTAL_HITS_CAP as i64));
        let rows = sql::all(connection, &sql, params, |row| {
            Ok((
                row.col::<Option<String>>(0)?,
                row.col::<i64>(1)?.max(0) as u64,
                [
                    row.col::<i64>(2)?,
                    row.col::<i64>(3)?,
                    row.col::<i64>(4)?,
                    row.col::<i64>(5)?,
                ],
            ))
        })
        .await?;
        let mut hits = 0;
        let mut flags = [0i64; 4];
        let mut folders = Vec::new();
        for (name, count, counts) in rows {
            match name {
                Some(name) => folders.push((name, count, counts)),
                None => {
                    hits = count;
                    flags = counts;
                }
            }
        }
        let mut refinements: Vec<Refinement> =
            ["is:unread", "is:flagged", "has:attach", LARGE_TOKEN]
                .into_iter()
                .zip(flags)
                .map(|(token, hits)| Refinement {
                    token: token.to_string(),
                    hits: hits.max(0) as u64,
                })
                .collect();

        folders.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        refinements.extend(
            folders
                .into_iter()
                .take(REFINE_FOLDERS)
                .map(|(name, hits, _)| Refinement {
                    token: format!("in:{}", quote_value(&name)),
                    hits,
                }),
        );
        Ok(CurrentScope { hits, refinements })
    }

    /// Two queries: the first selects only `m.id` and must stay simple enough for SQLite to keep
    /// the plan [`Plan::fetch_form`] depends on;
    /// adding the per-row correlated subqueries for every matching row cost several hundred ms on a
    /// broad match. Hydrating only the surviving ids pays that once per candidate.
    async fn fetch(
        &self,
        connection: &Connection,
        pool_size: u32,
        rank_by_relevance: bool,
        total_hits: u64,
        now: DateTime<Utc>,
    ) -> Result<Vec<Candidate>> {
        let scored = self
            .fetch_candidates(connection, pool_size, rank_by_relevance, total_hits, now)
            .await?;
        self.hydrate(connection, &scored).await
    }

    /// The scores come back here, not from `hydrate`: re-asking the indexes walked every posting a
    /// second time, 80 ms of a 100 ms budget.
    async fn fetch_candidates(
        &self,
        connection: &Connection,
        pool_size: u32,
        rank_by_relevance: bool,
        total_hits: u64,
        now: DateTime<Utc>,
    ) -> Result<Vec<(i64, f64)>> {
        let form = self.fetch_form(rank_by_relevance, total_hits);
        // Pool ordering: without a `GROUP BY` a message matched in both arms is ordered by its
        // better half and summed below. The age term is load-bearing: ordered by `bm25` alone,
        // every candidate for `invoice` on a real store was over a year old, so no later ranking
        // could surface a recent message.
        // Linear in years because this SQLite has no `exp` (see `POOL_AGE_WEIGHT_PER_YEAR`); `?` is
        // bound to now in milliseconds.
        let order_by = if rank_by_relevance {
            &format!(
                "-coalesce(hits.meta, 0.0) - coalesce(hits.body, 0.0) \
                 + {POOL_AGE_WEIGHT_PER_YEAR} * (? - {AGED_FROM}) / {MILLIS_PER_YEAR}"
            )
        } else {
            "m.received_at DESC"
        };
        // The probed path carries no scores on purpose: it is taken only for a match too broad to
        // rank, where `bm25` is near-uniform, and adding any column (even correlated subqueries)
        // loses the plan and costs half a second.
        let scores = match (self.has_match, form) {
            (true, Form::Driven) => "-hits.meta, -hits.body",
            _ => "NULL, NULL",
        };
        let mut params: Vec<turso::Value> = Vec::new();
        let sql = format!(
            "SELECT m.id, {scores} {from} WHERE {where_sql} ORDER BY {order_by} LIMIT ?",
            from = self.source_sql(form),
            where_sql = self.where_sql(form),
        );

        params.extend(self.params_for(form));
        // Bound here: parameters are positional and the `ORDER BY` sits between the `WHERE`'s and
        // the `LIMIT`'s; only when that ordering carries the term.
        if rank_by_relevance {
            params.push(turso::Value::Integer(now.timestamp_millis()));
        }
        // Twice the pool: the union can return the same message twice, and a message appears at
        // most once per index.
        params.push(turso::Value::Integer(
            i64::from(pool_size).saturating_mul(2),
        ));

        let mut statement = sql::statement(connection, &sql).await?;
        let rows = sql::mapped(&mut statement, params.clone(), |row| {
            let meta: Option<f64> = row.col(1)?;
            let body: Option<f64> = row.col(2)?;
            Ok((
                row.col::<i64>(0)?,
                meta.unwrap_or(0.0) + BODY_SCORE_WEIGHT * body.unwrap_or(0.0),
            ))
        })
        .await?;

        // Folded here, not with a `GROUP BY`, which would sort the match set. Matching in both
        // indexes is the best a message can do: `bm25` is negative-is-better, so each term pulls
        // the score down.
        let mut order: Vec<i64> = Vec::with_capacity(pool_size as usize);
        let mut scored: std::collections::HashMap<i64, f64> =
            std::collections::HashMap::with_capacity(pool_size as usize);
        for (id, score) in rows {
            match scored.entry(id) {
                std::collections::hash_map::Entry::Occupied(mut seen) => *seen.get_mut() += score,
                std::collections::hash_map::Entry::Vacant(empty) => {
                    empty.insert(score);
                    order.push(id);
                }
            }
        }
        order.truncate(pool_size as usize);
        Ok(order
            .into_iter()
            .map(|id| (id, scored.get(&id).copied().unwrap_or(0.0)))
            .collect())
    }

    /// `ids` is bounded by the candidate pool, so the `IN` list and per-row correlated subqueries
    /// are cheap whatever the match size. No FTS: the scores arrived with the ids.
    /// `hydrate_sql` is its own function so a test can hold its query plan still.
    fn hydrate_sql(&self, placeholders: &str) -> String {
        // Scoped to one account, affinity is that account's evidence about the sender; unified, it
        // is the person's, summed across every address they own (specs/005-contacts R2).
        let affinity = match self.account.account() {
            Some(_) => {
                "(SELECT s.times_seen FROM contact_sightings s
                    WHERE s.address_id = a.id AND s.account_id = ?)"
            }
            None => "(SELECT c.times_seen FROM contacts c WHERE c.id = a.contact_id)",
        };
        // One correlated lookup for the sender, not three: asking `recipients` for name, address
        // and normalized address per candidate was 384 ms of a 520 ms cold search.
        // The affinity probe compares against a plain joined column, never a nested subquery, or
        // SQLite cannot use it as an index key and walks every contact per candidate
        // (4.5 s for 320 matches, #746; `hydrate_probes_contacts_by_address_key` pins the plan).
        format!(
            "SELECT
                 sub.id, sub.thread_id, sub.mailbox_id, sub.subject, sub.received_at,
                 coalesce(named.name, sender.name) AS from_name, a.address AS from_address,
                 {affinity} AS sender_times_seen,
                 sub.preview, sub.aged_from
             FROM (SELECT
                     m.id, m.thread_id, m.mailbox_id, m.subject, m.received_at, m.preview,
                     {AGED_FROM} AS aged_from,
                     (SELECT r.id FROM recipients r
                        WHERE r.message_id = m.id AND r.kind = 'from'
                        ORDER BY r.position LIMIT 1) AS from_recipient
                   FROM messages m WHERE m.id IN ({placeholders})) sub
             LEFT JOIN recipients sender ON sender.id = sub.from_recipient
             LEFT JOIN addresses a ON a.id = sender.address_id
             LEFT JOIN contacts named
               ON named.id = a.contact_id AND named.state = 'live'",
        )
    }

    async fn hydrate(
        &self,
        connection: &Connection,
        scored: &[(i64, f64)],
    ) -> Result<Vec<Candidate>> {
        if scored.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = scored.iter().map(|(id, _)| *id).collect();

        // No `snippet()`: `message_bodies_fts` has no content by design (#407), so the excerpt is
        // cut by `postio_search::highlight::snippet` from the body the caller reads, which is the
        // text that was indexed.
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");

        let sql = self.hydrate_sql(&placeholders);

        let mut params = Vec::with_capacity(ids.len() + 1);
        if let Some(id) = self.account.account() {
            params.push(turso::Value::Integer(id.get()));
        }
        params.extend(ids.iter().map(|id| turso::Value::Integer(*id)));

        let mut statement = sql::statement(connection, &sql).await?;
        let by_id: std::collections::HashMap<i64, Candidate> =
            sql::mapped(&mut statement, params.clone(), |row| {
                let id: i64 = row.col(0)?;
                Ok((
                    id,
                    Candidate {
                        message_id: MessageId::new(id),
                        thread_id: row.col::<Option<i64>>(1)?.map(ThreadId::new),
                        mailbox_id: MailboxId::new(row.col(2)?),
                        subject: row.col(3)?,
                        received_at: postio_storage::repository::from_millis(row.col(4)?),
                        from_name: row.col(5)?,
                        from_address: row.col(6)?,
                        sender_times_seen: row.col::<Option<i64>>(7)?.unwrap_or(0),
                        preview: row.col(8)?,
                        aged_from: postio_storage::repository::from_millis(row.col(9)?),
                        bm25: 0.0,
                        snippet: String::new(),
                        score: 0.0,
                    },
                ))
            })
            .await?
            .into_iter()
            .collect();

        // `hydrate` has no `ORDER BY`: the caller's ordering lives in the pool's order.
        Ok(scored
            .iter()
            .filter_map(|(id, score)| {
                by_id.get(id).cloned().map(|mut candidate| {
                    candidate.bm25 = *score;
                    candidate
                })
            })
            .collect())
    }
}

/// By mailbox role, not id, so the scope means the same on every account and before any folder is
/// chosen (see [`Scope::Lists`]).
fn scope_condition(
    scope: Scope,
    account: AccountScope,
    names_a_folder: bool,
) -> Option<(String, Vec<turso::Value>)> {
    let role = scope_role(scope, names_a_folder)?;
    // Unified drops the account conjunct and nothing else, so "every account's inbox" is a
    // predicate removal, not a redefinition of "Inbox" (#186, ADR 0005 Q5a).
    Some(match account.account() {
        Some(id) => (
            format!("m.mailbox_id IN (SELECT id FROM mailboxes WHERE account_id = ? AND {role})"),
            vec![turso::Value::Integer(id.get())],
        ),
        None => (
            format!("m.mailbox_id IN (SELECT id FROM mailboxes WHERE {role})"),
            Vec::new(),
        ),
    })
}

/// Shared with [`corpus_complete_sql`], so the two cannot disagree about which folders a scope
/// means.
fn scope_role(scope: Scope, names_a_folder: bool) -> Option<&'static str> {
    Some(match scope {
        // "All mail" excludes drafts, junk and trash (#1523): a search is navigation. An `in:` in
        // the query lifts the exclusion, since it already confines the query to the folder named.
        Scope::AllMail if names_a_folder => return None,
        Scope::AllMail => "role NOT IN ('drafts', 'junk', 'trash')",
        Scope::Inbox => "role = 'inbox'",
        Scope::Lists => "role = 'regular'",
    })
}

/// Only an affirmative `in:`: `-in:trash` keeps the default exclusion and adds to it.
fn names_a_folder(query: &ParsedQuery) -> bool {
    query
        .filters()
        .any(|clause| !clause.negated && clause.filter.field() == postio_search::query::Field::In)
}

fn filter_condition(filter: &Filter, people: bool) -> (String, Vec<turso::Value>) {
    match filter {
        Filter::From(value) => with_people(
            fts_column_condition("sender", value),
            value,
            FROM_KINDS,
            people,
        ),
        Filter::To(value) => with_people(
            fts_column_condition("recipients", value),
            value,
            TO_KINDS,
            people,
        ),
        Filter::Subject(value) => fts_column_condition("subject", value),
        // Resolved in SQL: a name that resolves to nothing is an empty `IN` set and matches
        // nothing, never everything, which is what dropping the predicate would mean.
        Filter::Account(value) => (
            "m.account_id IN (SELECT id FROM accounts \
             WHERE lower(display_name) = lower(?) OR lower(address) = lower(?))"
                .to_string(),
            vec![
                turso::Value::Text(value.clone()),
                turso::Value::Text(value.clone()),
            ],
        ),
        Filter::In(value) => (
            "m.mailbox_id IN (SELECT id FROM mailboxes \
             WHERE lower(name) = lower(?) OR lower(path) = lower(?) OR role = lower(?))"
                .to_string(),
            vec![
                turso::Value::Text(value.clone()),
                turso::Value::Text(value.clone()),
                turso::Value::Text(value.clone()),
            ],
        ),
        // Resolved against `recipients` by exact address, not full text. An unresolvable group or
        // address is an empty set and matches nothing, as the arms around this one do.
        Filter::With(addresses) => {
            let placeholders = vec!["?"; addresses.len()].join(", ");
            (
                format!(
                    "m.id IN (SELECT r.message_id FROM recipients r \
                     JOIN addresses a ON a.id = r.address_id \
                     WHERE r.kind IN ('from', 'to', 'cc', 'bcc') \
                       AND a.address_normalized IN ({placeholders}))"
                ),
                addresses
                    .iter()
                    .map(|address| turso::Value::Text(address.to_lowercase()))
                    .collect(),
            )
        }
        // A member is a person: the set is every address any live member owns, matched by
        // `recipients.address_id` key. A deleted member widens nothing.
        Filter::Group(value) => (
            format!("m.id IN ({GROUP_SET})"),
            vec![turso::Value::Text(value.clone())],
        ),
        // ADR 0025 Q2, the one operator that is not an FTS `MATCH`: header values are structured
        // and a tokenizer would split them at the characters that made them meaningful.
        // The name is compared for equality, never as a substring (`header:x-mail` must not find
        // `X-Mailer`).
        Filter::Header { name, value } => match value {
            None => (
                "EXISTS (SELECT 1 FROM message_headers h \
                  WHERE h.content_id = m.content_id AND h.name = ?)"
                    .to_string(),
                vec![turso::Value::Text(name.clone())],
            ),
            // `LIKE` folds ASCII case (ADR 0025 Q6); a non-ASCII value matches in the case it was
            // stored, as SQLite's `lower()` is ASCII-only too.
            Some(value) => (
                "EXISTS (SELECT 1 FROM message_headers h \
                  WHERE h.content_id = m.content_id AND h.name = ? \
                    AND h.value LIKE '%' || ? || '%' ESCAPE '\\')"
                    .to_string(),
                vec![
                    turso::Value::Text(name.clone()),
                    turso::Value::Text(escape_like(value)),
                ],
            ),
        },
        Filter::Filename(value) => fts_column_condition("filenames", value),
        Filter::List(value) => fts_column_condition("list_id", value),
        Filter::HasAttachment => ("m.has_attachments = 1".to_string(), Vec::new()),
        // The join through `message_labels` keeps it to the message's own account. Case-insensitive
        // like the name's uniqueness (`idx_labels_account_name`); a name no label has matches
        // nothing.
        Filter::Label(value) => (
            "EXISTS (SELECT 1 FROM message_labels ml \
              JOIN labels l ON l.id = ml.label_id \
              WHERE ml.message_id = m.id AND lower(l.name) = lower(?))"
                .to_string(),
            vec![turso::Value::Text(value.clone())],
        ),
        Filter::HasAction => (
            "EXISTS (SELECT 1 FROM markers k \
              WHERE k.message_id = m.id AND k.dismissed_at IS NULL)"
                .to_string(),
            Vec::new(),
        ),
        Filter::Is(state) => {
            use postio_search::query::State;
            match state {
                State::Unread => ("m.seen = 0".to_string(), Vec::new()),
                State::Read => ("m.seen = 1".to_string(), Vec::new()),
                State::Flagged => ("m.flagged = 1".to_string(), Vec::new()),
                // NULL is "not known yet", and a NULL comparison is no match: mail whose headers
                // nothing has read is neither.
                State::Bulk => (
                    format!(
                        "(m.unsubscribe_offered = 1 OR (m.automation & {}) <> 0)",
                        postio_model::promoted::PRECEDENCE
                    ),
                    Vec::new(),
                ),
                State::Automated => (
                    format!(
                        "(m.automation & {}) <> 0",
                        postio_model::promoted::AUTO_SUBMITTED
                    ),
                    Vec::new(),
                ),
            }
        }
        Filter::After(date) => (
            "m.received_at >= ?".to_string(),
            vec![turso::Value::Integer(day_start_millis(*date))],
        ),
        Filter::Before(date) => (
            "m.received_at < ?".to_string(),
            vec![turso::Value::Integer(day_start_millis(*date))],
        ),
        Filter::Larger(bytes) => (
            "m.size >= ?".to_string(),
            vec![turso::Value::Integer(*bytes as i64)],
        ),
        Filter::Smaller(bytes) => (
            "m.size <= ?".to_string(),
            vec![turso::Value::Integer(*bytes as i64)],
        ),
        Filter::AnyOf(set) => {
            let mut sql = Vec::with_capacity(set.members().len());
            let mut params = Vec::new();
            for member in set.members() {
                let (condition, mut values) = filter_condition(member, people);
                sql.push(format!("({condition})"));
                params.append(&mut values);
            }
            (format!("({})", sql.join(" OR ")), params)
        }
    }
}

/// Whole tokens, not substrings: a `LIKE '%value%'` per message blew the `<100 ms` budget on a
/// large mailbox (postio-y47); querying the indexed column is one lookup.
/// The column is named inside the query (`sender:"ada@…"`): `fts_match(sender, ?)` with fewer
/// columns than the index covers does not use the index and is true when any one word is present,
/// so `from:ada@example.com` found mail from anyone at `example.com` (#1699).
fn fts_column_condition(column: &str, value: &str) -> (String, Vec<turso::Value>) {
    let (contents, params) = fts_column_contents(column, value);
    (format!("m.content_id IN ({contents})"), params)
}

fn fts_column_set(column: &str, value: &str) -> (String, Vec<turso::Value>) {
    let (contents, params) = fts_column_contents(column, value);
    (
        format!(
            "SELECT m.id AS message_id FROM ({contents}) d
               CROSS JOIN messages m INDEXED BY idx_messages_content
                 ON m.content_id = d.content_id"
        ),
        params,
    )
}

fn fts_column_contents(column: &str, value: &str) -> (String, Vec<turso::Value>) {
    (
        "SELECT content_id FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, ?)"
            .to_owned(),
        vec![turso::Value::Text(format!(
            "{column}:{}",
            fts_literal(value)
        ))],
    )
}

/// Matched against `recipients.address_id` by key rather than by comparing strings. A deleted
/// member widens nothing.
const GROUP_SET: &str = "SELECT r.message_id FROM recipients r \
     WHERE r.kind IN ('from', 'to', 'cc', 'bcc') \
       AND r.address_id IN ( \
         SELECT a.id FROM contact_groups g \
         JOIN contact_group_members gm ON gm.group_id = g.id \
         JOIN contacts c ON c.id = gm.contact_id AND c.state = 'live' \
         JOIN addresses a ON a.contact_id = c.id \
         WHERE lower(g.name) = lower(?))";

/// `label:` and `has:action` are cheap `EXISTS` probes per row; as sets they let a conversation
/// search walk only the messages they keep when nothing else narrows it.
fn id_set(filter: &Filter, people: bool) -> Option<(String, Vec<turso::Value>)> {
    Some(match filter {
        // `UNION` rather than `UNION ALL`: a set may drive the walk (`relaxations`), which must see
        // each message once.
        Filter::AnyOf(set) => {
            let mut arms = Vec::with_capacity(set.members().len());
            let mut params = Vec::new();
            for member in set.members() {
                let (sql, mut values) = id_set(member, people)?;
                arms.push(format!("SELECT message_id FROM ({sql})"));
                params.append(&mut values);
            }
            (arms.join(" UNION "), params)
        }
        Filter::From(value) => {
            set_with_people(fts_column_set("sender", value), value, FROM_KINDS, people)
        }
        Filter::To(value) => {
            set_with_people(fts_column_set("recipients", value), value, TO_KINDS, people)
        }
        Filter::Subject(value) => fts_column_set("subject", value),
        Filter::Filename(value) => fts_column_set("filenames", value),
        Filter::List(value) => fts_column_set("list_id", value),
        Filter::Group(value) => (
            GROUP_SET.to_string(),
            vec![turso::Value::Text(value.clone())],
        ),
        Filter::Label(value) => (
            "SELECT ml.message_id FROM labels l \
               JOIN message_labels ml ON ml.label_id = l.id \
              WHERE lower(l.name) = lower(?)"
                .to_string(),
            vec![turso::Value::Text(value.clone())],
        ),
        Filter::HasAction => (
            "SELECT message_id FROM markers WHERE dismissed_at IS NULL".to_string(),
            Vec::new(),
        ),
        _ => return None,
    })
}

const FROM_KINDS: &str = "'from'";
const TO_KINDS: &str = "'to', 'cc', 'bcc'";

/// `None` for a value that names neither (a phrase of several words is left to the header).
fn people_addresses(value: &str) -> Option<(String, Vec<turso::Value>)> {
    let value = value.trim();
    if let Some(group) = value.strip_prefix('@') {
        return Some((
            "SELECT a.id FROM contact_groups g \
               JOIN contact_group_members gm ON gm.group_id = g.id \
               JOIN contacts c ON c.id = gm.contact_id AND c.state = 'live' \
               JOIN addresses a ON a.contact_id = c.id \
              WHERE lower(g.name) = lower(?)"
                .to_string(),
            vec![turso::Value::Text(group.to_string())],
        ));
    }
    if value.is_empty() || value.contains(char::is_whitespace) {
        return None;
    }
    Some((
        "SELECT a.id FROM contact_terms t \
           JOIN contacts c ON c.id = t.contact_id AND c.state = 'live' \
           JOIN addresses a ON a.contact_id = c.id \
          WHERE t.term = ?"
            .to_string(),
        vec![turso::Value::Text(value.to_lowercase())],
    ))
}

/// Explicit syntax, never a word a header holds.
fn names_a_group(value: &str) -> bool {
    value.trim_start().starts_with('@')
}

fn with_people(
    (header, mut values): (String, Vec<turso::Value>),
    value: &str,
    kinds: &str,
    people: bool,
) -> (String, Vec<turso::Value>) {
    let Some((addresses, mut more)) = (people || names_a_group(value))
        .then(|| people_addresses(value))
        .flatten()
    else {
        return (header, values);
    };
    let by_person = format!(
        "m.id IN (SELECT r.message_id FROM recipients r \
                   WHERE r.kind IN ({kinds}) AND r.address_id IN ({addresses}))"
    );
    if value.trim_start().starts_with('@') {
        return (by_person, more);
    }
    values.append(&mut more);
    (format!("(({header}) OR {by_person})"), values)
}

fn set_with_people(
    (header, mut values): (String, Vec<turso::Value>),
    value: &str,
    kinds: &str,
    people: bool,
) -> (String, Vec<turso::Value>) {
    let Some((addresses, mut more)) = (people || names_a_group(value))
        .then(|| people_addresses(value))
        .flatten()
    else {
        return (header, values);
    };
    let by_person = format!(
        "SELECT r.message_id FROM recipients r \
          WHERE r.kind IN ({kinds}) AND r.address_id IN ({addresses})"
    );
    if value.trim_start().starts_with('@') {
        return (by_person, more);
    }
    values.append(&mut more);
    (
        format!("SELECT message_id FROM ({header}) UNION {by_person}"),
        values,
    )
}

/// `%` and `_` are wildcards to `LIKE` and turn up in header values (`boundary=__part_1`, `100%`);
/// without this `header:x-spam-status=100%` would match any `X-Spam-Status`.
fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// A folder called `Old Mail` must become `in:"Old Mail"` or the parser reads `in:Old` and a stray
/// word.
fn quote_value(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', ""))
    } else {
        value.to_string()
    }
}

fn day_start_millis(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight always exists")
        .and_utc()
        .timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(days_ago: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 23, 12, 0, 0).unwrap() - chrono::Duration::days(days_ago)
    }

    fn matching_plan() -> Plan {
        Plan {
            conditions: Vec::new(),
            params: Vec::new(),
            account: AccountScope::Unified,
            has_match: true,
            widened: false,
            match_param: Some(turso::Value::Text("invoice".to_owned())),
            body_match_param: Some(turso::Value::Text("invoice".to_owned())),
            sets: Vec::new(),
        }
    }

    #[test]
    fn coverage_is_the_share_of_terms_the_subject_and_sender_say() {
        let terms = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            coverage(
                &terms(&["voice", "lessons", "zoom", "link"]),
                "Re: Voice Lessons Ada ada@example.com"
            ),
            0.5
        );
        assert_eq!(
            coverage(
                &terms(&["hannah", "invoice"]),
                "Lapiduz 9/26 Invoices Hannah's Music Studio"
            ),
            1.0
        );
        assert_eq!(coverage(&terms(&["bingo"]), "Re: Voice Lessons"), 0.0);
        assert_eq!(coverage(&[], "anything"), 0.0);
    }

    #[test]
    fn near_equal_text_matches_are_one_band_led_by_the_best() {
        assert_eq!(
            text_bands(&[-18.19, -17.61, -17.19, -6.9, -6.5, -2.0]),
            vec![-18.19, -18.19, -18.19, -6.9, -6.9, -2.0]
        );
    }

    #[test]
    fn a_band_is_measured_from_its_best_so_it_cannot_drift() {
        let bands = text_bands(&[-10.0, -9.5, -9.1, -8.6]);
        assert_eq!(bands, vec![-10.0, -10.0, -10.0, -8.6]);
    }

    #[test]
    fn in_one_band_the_newer_message_ranks_first() {
        let now = Utc::now();
        let at = |days: i64| now - chrono::TimeDelta::days(days);
        let bands = text_bands(&[-18.19, -17.61, -17.19]);
        let older = rank_score(bands[0], at(21), now, 5);
        let newer = rank_score(bands[1], at(12), now, 5);
        let newest = rank_score(bands[2], at(6), now, 5);
        assert!(
            newest < newer && newer < older,
            "lower is better: {newest} {newer} {older}"
        );
    }

    #[test]
    fn recency_can_still_reorder_mail_that_is_years_old() {
        let now = at(0);
        let five_years = rank_score(-6.0, at(1825), now, 0);
        let eighteen_years = rank_score(-6.0, at(6570), now, 0);
        assert!(
            five_years < eighteen_years,
            "five-year-old mail must outrank eighteen-year-old at equal bm25: \
             {five_years} against {eighteen_years}"
        );

        assert!(
            eighteen_years - five_years > 0.40,
            "the gap between five and eighteen years is {}, too small to move \
             anything against a bm25 spread of 0.80",
            eighteen_years - five_years
        );
    }

    #[test]
    fn a_much_better_match_still_beats_a_more_recent_one() {
        let now = at(0);
        let strong_and_old = rank_score(-12.0, at(3650), now, 0);
        let weak_and_fresh = rank_score(-6.0, at(0), now, 0);
        assert!(
            strong_and_old < weak_and_fresh,
            "a six-point better bm25 must beat freshness: {strong_and_old} \
             against {weak_and_fresh}"
        );
    }

    #[test]
    fn the_join_form_follows_the_match_size_not_the_ordering() {
        let plan = matching_plan();

        assert_eq!(plan.fetch_form(true, 500), Form::Driven);

        // Ordering and join form are separate questions: switching the form on the ordering
        // threshold made a term scattered through old mail scan deep (`invoice` at 3,843 hits took
        // 972 ms probed against 142 ms driven).
        assert_eq!(plan.fetch_form(false, 3_843), Form::Driven);
        assert_eq!(plan.fetch_form(false, PROBED_FORM_LIMIT), Form::Driven);

        assert_eq!(plan.fetch_form(false, PROBED_FORM_LIMIT + 1), Form::Probed);
        assert_eq!(plan.fetch_form(false, TOTAL_HITS_CAP), Form::Probed);
    }

    #[test]
    fn ranking_by_relevance_always_drives_from_the_match() {
        let plan = matching_plan();
        for hits in [0, 1, RANK_BY_RELEVANCE_LIMIT, TOTAL_HITS_CAP] {
            assert_eq!(
                plan.fetch_form(true, hits),
                Form::Driven,
                "relevance ranking must drive from the match at {hits} hits"
            );
        }
    }

    #[test]
    fn a_more_recent_message_ranks_first_at_equal_relevance() {
        let now = at(0);
        let older = rank_score(-1.0, at(30), now, 0);
        let newer = rank_score(-1.0, at(1), now, 0);
        assert!(newer < older, "newer: {newer}, older: {older}");
    }

    #[test]
    fn a_frequent_sender_ranks_first_at_equal_relevance_and_recency() {
        let now = at(0);
        let stranger = rank_score(-1.0, at(10), now, 0);
        let regular = rank_score(-1.0, at(10), now, 50);
        assert!(regular < stranger);
    }

    /// The plan is pinned because a timing assertion at that scale is a bench's job
    /// (`search_budget.rs` seeds contacts for it).
    #[tokio::test]
    async fn hydrate_probes_contacts_by_address_key() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        crate::index::ensure_schema(&connection)
            .await
            .expect("schema");

        let query = postio_search::parse("invoice", at(0).date_naive());
        let request = SearchRequest {
            account: AccountScope::Account(postio_model::AccountId::new(1)),
            query: &query,
            scope: postio_search::facets::Scope::AllMail,
            limit: 200,
            order: postio_search::ResultOrder::Relevance,
        };
        let plan = Plan::build(&request);

        let sql = plan.hydrate_sql("?, ?, ?");
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .await
            .expect("prepare the hydrate statement");
        let steps: Vec<String> = sql::mapped(
            &mut statement,
            postio_storage::bind![1i64, 10i64, 11i64, 12i64],
            |row| row.col(3),
        )
        .await
        .expect("explain");

        assert!(
            steps.iter().any(|step| step.contains("contact_sightings")
                && step.contains("address_id=?")
                && step.contains("account_id=?")),
            "the affinity probe is not keyed on the address and account; plan:\n{steps:#?}"
        );
        assert!(
            !steps
                .iter()
                .any(|step| step.starts_with("SCAN") && step.contains("contact")),
            "a contacts table is being scanned per candidate; plan:\n{steps:#?}"
        );
    }

    #[tokio::test]
    async fn the_probed_form_collapses_an_exact_copy() {
        use postio_model::{EmailAddress, Mailbox, MailboxRole, Message, RfcMessageId};
        use postio_storage::repository::{MailboxRepository, MessageRepository};

        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        crate::index::ensure_schema(&connection)
            .await
            .expect("schema");
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut sent = Mailbox::new(account.id, "Sent Messages", Some('/'));
        sent.role = MailboxRole::Sent;
        MailboxRepository::new(&connection)
            .create(&mut sent)
            .await
            .expect("a folder");
        let mut ids = Vec::new();
        for (mailbox, body) in [(inbox, None), (sent.id, Some("the nebula, mapped"))] {
            let mut message = Message::new(account.id, mailbox, at(1));
            message.rfc_message_id = Some(RfcMessageId::new("<survey-1@example.com>"));
            message.date = Some(at(1));
            message.size = 4_096;
            message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
            message.subject = Some("Nebula survey".to_owned());
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("file it");
            if let Some(body) = body {
                crate::index::index_body(&connection, message.id.get(), Some(body))
                    .await
                    .expect("index its body");
            }
            ids.push(message.id.get());
        }

        for (text, expected) in [
            ("nebula", ids[0]),
            ("nebula after:2026-01-01", ids[0]),
            ("mapped after:2026-01-01", ids[1]),
        ] {
            let query = postio_search::parse(text, at(0).date_naive());
            let request = SearchRequest {
                account: AccountScope::Account(account.id),
                query: &query,
                scope: postio_search::facets::Scope::AllMail,
                limit: 10,
                order: postio_search::ResultOrder::Newest,
            };
            let plan = Plan::build(&request);
            assert_eq!(plan.fetch_form(false, PROBED_FORM_LIMIT + 1), Form::Probed);
            let found: Vec<i64> = plan
                .fetch_candidates(&connection, 10, false, PROBED_FORM_LIMIT + 1, at(0))
                .await
                .expect("the probed pool")
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            assert_eq!(found, [expected], "{text}");
        }
    }

    #[test]
    fn a_much_better_text_match_still_wins_over_recency_and_affinity() {
        let now = at(0);
        let weak_but_boosted = rank_score(-0.1, at(0), now, 1000);
        let strong_but_unboosted = rank_score(-20.0, at(365), now, 0);
        assert!(strong_but_unboosted < weak_but_boosted);
    }
}
