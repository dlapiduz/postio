//! Query execution: combining structured filters with the full-text index,
//! ranking the results, and cutting snippets out of the match.
//!
//! [`search`] is the one entry point. It takes a [`ParsedQuery`] (see
//! [`postio_search::parse`]) and an account to search within, and returns
//! [`SearchResults`]: a page of ranked, snippeted [`SearchHit`]s plus the
//! total hit count and how long the search took — both of which the canvas
//! 2b readout ("14 hits · 11 ms") shows live.
//!
//! # Ranking
//!
//! The index's `fts_score` ranks relevance alone. That is not
//! the whole story a mail search wants: a five-year-old message that happens
//! to say "invoice" once should not usually outrank one from yesterday, and
//! a sender the user emails constantly deserves a nudge. [`rank_score`] folds
//! in both as a small negative adjustment (recency and sender affinity only
//! ever help a candidate, never hurt it), and is a pure function precisely so
//! the orderings it produces can be tested without a database — see its own
//! tests. [`search`] fetches a bounded candidate pool ordered by `bm25` (or,
//! past [`RANK_BY_RELEVANCE_LIMIT`] matches, by recency — seeing why is the
//! rest of this module's story), re-ranks that pool in Rust, and truncates to
//! the page size: reordering a few hundred rows in memory is cheap, and it
//! keeps the scoring logic out of SQL entirely.

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

/// How many candidates `search` pulls out of SQL before re-ranking in Rust,
/// as a multiple of the requested page size.
const CANDIDATE_POOL_MULTIPLIER: u32 = 5;

/// The floor on the candidate pool, so a `limit` of 1 or 2 still gives the
/// ranker enough rows to find a better match further down the SQL ordering.
const CANDIDATE_POOL_MIN: u32 = 200;

/// Above this many matches, `fetch` orders by recency instead of `bm25`. See
/// the comment in [`search`] for why: ranking every match in a very broad
/// query is not affordably fast, and this is the threshold past which it
/// stops being worth trying.
const RANK_BY_RELEVANCE_LIMIT: u64 = 2_000;

/// Past how many matches the probed shape becomes the cheaper one.
///
/// **Ordering and join form are separate decisions, and conflating them was a
/// bug.** Past [`RANK_BY_RELEVANCE_LIMIT`] a match is too broad to rank, so it
/// orders by recency — but that says nothing about which side should drive the
/// join, and until this constant existed the same threshold decided both.
///
/// The probed shape walks `messages` newest-first and asks each row "did you
/// match?", so its cost is set by how *recent* the matches are, not how many
/// there are. That is fast for a word in most of the mailbox and slow for one
/// scattered through old mail — and "too broad to rank" catches both.
///
/// Measured against a real 82,132-message store, `LIMIT 100`, 16 MiB cache:
///
/// ```text
/// term           hits    probed    driven by the match
/// invoice       3,843     972ms                  142ms
/// meeting       3,646     511ms                  119ms
/// unsubscribe   8,137     167ms                  182ms
/// the          10,000+     74ms                1,257ms
/// ```
///
/// The two shapes cross over near 8,000, and the band between 2,000 and there
/// was taking the wrong one — `invoice` at seven times the cost of the
/// alternative. Above it the probe is worth an order of magnitude, which is
/// why the answer is a second threshold rather than deleting the shape.
///
/// The count saturates at `TOTAL_HITS_CAP` (10,000), so a term in half the
/// mailbox reports 10,000 and lands here, which is where it belongs.
const PROBED_FORM_LIMIT: u64 = 8_000;

/// [`CANDIDATE_POOL_MULTIPLIER`], for a recency-ordered fetch. See the
/// comment in [`search`] on `pool_size`.
const RECENCY_POOL_MULTIPLIER: u32 = 2;
/// [`CANDIDATE_POOL_MIN`], for a recency-ordered fetch.
const RECENCY_POOL_MIN: u32 = 50;

/// Recency's weight in [`rank_score`], relative to the match term's native
/// scale.
///
/// Raised with the half-life below, and the two go together: a heavier weight
/// on a term that is zero for every candidate changes nothing.
///
/// # Re-measured against `fts_score`, and deliberately unchanged
///
/// This number was calibrated against `bm25()`, which is gone. Measured on
/// `index_suite::ranking_weights`' corpus — 2,000 messages, term frequency
/// crossed with document length, which is what BM25 is a function of:
///
/// ```text
///                        bm25 (a real store)   fts_score (the fixture)
/// spread                 0.80 over the top 40  0.20 over the whole match set
/// the best forty         separated             all one value
/// ```
///
/// About a quarter of the range, and the top of a result set is routinely
/// tied outright — the score is a function of frequency and length, and mail
/// is full of near-identical subjects. So the blend is **more recency-led
/// than it was**, and no weight avoids that: 0.20 is the entire relevance
/// range, and a recency term small enough to fit under it cannot separate
/// last week from last year.
///
/// Unchanged, then, as a decision rather than a carry-over. #1216's complaint
/// was search surfacing very old mail, and recency leading is what that asked
/// for. What still has to hold is the narrow case — a *clearly* better match
/// beats a *small* recency difference — which
/// `newest_order_answers_in_date_order_however_the_ranking_disagrees` pins
/// with a 5x-density contrast five hours apart, and `ranking_weights` pins in
/// the units it measures.
const RECENCY_WEIGHT: f64 = 3.0;
/// The age, in days, at which the recency boost has halved.
///
/// **Two years, and it was fourteen days.** Measured against a real store, the
/// forty best `bm25` matches for `invoice` had a median age of 6,687 days and
/// every one of them was over a year old. An exponential with a fourteen-day
/// half-life is zero to four decimal places across that whole range, so
/// recency moved those scores by 0.00 and `bm25` alone decided the order --
/// which is exactly the complaint that prompted this: search pulling up very
/// old mail.
///
/// The shape matters more than the weight. Measured spread across those same
/// candidates, which is what decides whether a term can reorder anything:
///
/// ```text
/// half-life        spread    x weight   against a bm25 spread of 0.80
/// 14 days           0.000        0.00   cannot reorder
/// 1 year            0.110        0.33   cannot
/// 2 years           0.331        0.99   can
/// 5 years           0.569        1.71   can
/// ```
///
/// Two years keeps a useful gradient over the range mail actually lives in --
/// a month is 0.97, a year 0.71, five years 0.21, eighteen years 0.002 -- and
/// still separates last week from last month, which a five-year half-life
/// starts to flatten.
const RECENCY_HALF_LIFE_DAYS: f64 = 730.0;

/// Age's weight in the *pool* ordering, per year, in `bm25` units.
///
/// A separate term from [`RECENCY_WEIGHT`] because it answers a separate
/// question, and missing that is why raising the weight alone would have
/// changed nothing. `rank_score` reorders the candidates it is given; this
/// decides which candidates there are. The pool was ordered by `bm25` alone,
/// so on the store above every one of the 400 it handed to `rank_score` was
/// over a year old -- and no ranking function can surface a recent message
/// that never entered the pool.
///
/// **Linear, not exponential, and not by choice.** The store engine has no
/// math functions -- `exp`, `ln` and `pow` are all absent -- so the ordering
/// can only use arithmetic. Linear in years is what that allows, and it has
/// the virtue of never saturating: it keeps separating eighteen years from
/// five where an exponential has long since flattened.
///
/// At 0.25 a year of age costs a quarter of a `bm25` point, so five years
/// costs 1.25 -- about one and a half times the entire spread of the top forty
/// matches. An old message has to be substantially the better match to beat a
/// recent one, which is the intent, rather than being excluded outright.
const POOL_AGE_WEIGHT_PER_YEAR: f64 = 0.25;

/// How far below a better text match a candidate may score and still count
/// as just as good a match: 10%.
///
/// The text score separates near-identical mail by noise. Three invoices
/// from one template scored -18.2, -17.6 and -17.2 for "Hannah invoice" on
/// a real store: a few percent, from a longer greeting or a name said
/// twice, and it outweighed nine days of recency, so the oldest came first.
/// Within this tolerance the matches are one band, and recency and the
/// sender decide; a match clearly better than another still wins outright.
const TEXT_TIE: f64 = 0.10;

/// The tolerance among messages whose subject and sender say *every* word
/// of the query: 50%.
///
/// They are each about what was asked -- one sender's newsletters found by
/// its name, one studio's invoices -- and a person reads them as a dated
/// series, newest first. Their scores still differ by more than
/// [`TEXT_TIE`] when, say, one of the sender's addresses also spells a word
/// of the query. Twice as good a match still leads.
const SAID_TIE: f64 = 0.50;

/// Each score in `bm25` (lower is better) replaced by the best of its band:
/// the candidates within [`TEXT_TIE`] of a better one. A band is measured
/// from its best, never from its last member, so a run of small steps
/// cannot drift a weak match into a strong one's band.
pub fn text_bands(bm25: &[f64]) -> Vec<f64> {
    bands(bm25, TEXT_TIE)
}

/// [`text_bands`], with the tolerance named: each score replaced by the
/// best of the run within `tie` of it.
fn bands(bm25: &[f64], tie: f64) -> Vec<f64> {
    let mut order: Vec<usize> = (0..bm25.len()).collect();
    order.sort_by(|a, b| bm25[*a].total_cmp(&bm25[*b]));
    let mut banded = bm25.to_vec();
    let mut best: Option<f64> = None;
    for index in order {
        let score = bm25[index];
        let lead = match best {
            // Scores are negative, larger in magnitude when better: within
            // the tolerance means at least (1 - TEXT_TIE) of the best's size.
            Some(lead) if lead < 0.0 && score <= lead * (1.0 - tie) => lead,
            _ => score,
        };
        best = Some(lead);
        banded[index] = lead;
    }
    banded
}

/// How much of the query a message's subject and sender say, from 0 to 1:
/// the share of `terms` whose every word begins a word of `said`.
///
/// What separates the message *about* something from one that mentions it
/// in passing, when their scores tie: the free-text score credits subject
/// and sender only when every term is there, so "zoom link voice lessons"
/// gave "Re: Voice Lessons" no credit for its subject at all. A prefix,
/// so "invoice" covers "invoices" and "hannah" covers "Hannah's".
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

/// How well a sender is known, in four steps of [`rank_score`]'s affinity:
/// coarse on purpose, so a sender's two addresses seen four and five times
/// are one step, while a correspondent seen eighty times is not a stranger.
fn known(times_seen: i64) -> u8 {
    let affinity = (1.0 + times_seen.max(0) as f64).ln() / (1.0 + 100f64).ln();
    (affinity.min(1.0) * 4.0).floor().min(3.0) as u8
}

/// Where the relevance order puts one candidate, once it is scored: what
/// [`search`] sorts its pool by, and what a conversation search sorts its
/// pool by with the match count in between ([`RelevanceKey::order`]).
#[derive(Debug, Clone, Copy)]
struct RelevanceKey {
    /// The best score of its band ([`TEXT_TIE`], or [`SAID_TIE`] among those
    /// whose subject and sender say every word).
    band: f64,
    /// How much of the query its subject and sender say ([`coverage`]).
    said: f64,
    /// How well its sender is known, coarsely ([`known`]).
    known: u8,
    aged_from: DateTime<Utc>,
    /// Its [`rank_score`].
    score: f64,
}

impl RelevanceKey {
    /// Better first. `between` decides what the subject and sender leave
    /// tied, before the sender and the age do: [`search`] passes `Equal`,
    /// a conversation search how many of each one's messages matched.
    fn order(&self, other: &Self, between: std::cmp::Ordering) -> std::cmp::Ordering {
        // In a band: what the subject and sender say, then how well the
        // sender is known in coarse steps (a correspondent of eighty
        // letters still leads a stranger, but four sightings against
        // five is noise), then the newer.
        self.band
            .total_cmp(&other.band)
            .then(other.said.total_cmp(&self.said))
            .then(between)
            .then(other.known.cmp(&self.known))
            .then(other.aged_from.cmp(&self.aged_from))
            .then(self.score.total_cmp(&other.score))
    }
}

/// Scores each candidate ([`rank_score`] over its text band) and says where
/// the relevance order puts it, in the candidates' own order.
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
    // Scores within [`TEXT_TIE`] of each other are one band: as good
    // an answer as each other, so within it the message whose subject
    // and sender say more of the query leads, then the newer one.
    // Recency and affinity are too weak to order recent mail on their
    // own -- a week is 0.02, and a sender seen four times rather than
    // five is 0.04 -- so a sender's own newsletters came back in no
    // order a person could see.
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
    // Those that say all of it are banded among themselves, more
    // loosely ([`SAID_TIE`]); the rest by [`TEXT_TIE`].
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

/// How old a message is, to the ranking: its own `Date`, when that is
/// earlier than the server's arrival date, else the arrival date.
///
/// `received_at` is when the message arrived *in its folder*. A move, an
/// archive or a resync files it again and the server dates it again, so
/// last month's invoice archived this morning was ranked as this morning's
/// mail and beat this week's. The `Date` header is the sender's claim, and
/// taken only when it is the earlier: a claim of a future date can never
/// lift a message, and arrival still bounds anything with no header.
const AGED_FROM: &str = "CASE WHEN m.date IS NOT NULL AND m.date < m.received_at \
     THEN m.date ELSE m.received_at END";

/// Milliseconds in a year, for the pool ordering's age term.
const MILLIS_PER_YEAR: f64 = 31_557_600_000.0;
/// Sender affinity's weight in [`rank_score`].
///
/// Unchanged for the reason [`RECENCY_WEIGHT`] gives, and it is the smaller
/// risk of the two: affinity is bounded in `[0, 1)` before weighting and only
/// separates correspondents, so at worst it orders a tie by who writes most.
const SENDER_WEIGHT: f64 = 1.0;

/// A search over one account's mail.
#[derive(Debug, Clone, Copy)]
pub struct SearchRequest<'a> {
    /// Which accounts to search: one, or all of them.
    ///
    /// Was a bare `AccountId` — search never crossed accounts — until #186.
    /// Orthogonal to [`scope`](Self::scope), which is the *role* tri-tab, so
    /// that "this account's inbox" and "every account's inbox" are both
    /// askable. Folding the two into one enum would have made them mutually
    /// exclusive; see ADR 0005 Q5.
    pub account: AccountScope,
    /// The already-parsed query. See [`postio_search::parse`].
    pub query: &'a ParsedQuery,
    /// Which slice of the account to look in.
    ///
    /// The canvas' left column, and a constraint the query text never
    /// carries: switching scope must not mean editing what was typed. See
    /// [`Scope`].
    pub scope: Scope,
    /// How many hits to return, at most.
    pub limit: u32,
    /// Ranked, or in plain date order (#499).
    ///
    /// `Newest` takes the recency-ordered fetch unconditionally — the same
    /// path a too-broad-to-rank query already takes — and skips the re-rank,
    /// so the answer is exactly a mailbox's own order rather than "mostly
    /// recency, nudged by affinity".
    pub order: postio_search::ResultOrder,
}

/// Runs a search and returns a ranked page of results.
///
/// `now` is the reference clock for the recency boost, taken as a parameter
/// for the same reason [`postio_search::parse`] takes `today`: it keeps
/// ranking a pure, reproducible function of its inputs.
pub async fn search(
    connection: &Connection,
    request: &SearchRequest<'_>,
    now: DateTime<Utc>,
) -> Result<SearchResults> {
    let exact = search_as(connection, request, now, &Default::default()).await?;
    // The forgiving search the command bar asks for (ADR 0037, as amended):
    // the words as typed first, and only when they do not fill the page,
    // the words near them -- a plural, an unfinished word, a misspelling --
    // after every exact match. A rule's query is never forgiving.
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

/// [`search`], once: exactly, or with each word `near` names read as any of
/// the words near it ([`near_words`]).
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
    // A term matched by most of a large mailbox has no cheap true top-K by
    // `bm25`: FTS5's incremental top-K scan only pays off when few enough
    // documents match that it can prove the rest can't beat what it already
    // has, and a term this broad gives it nothing to prove that with — the
    // "common word" shape of postio-y47's benchmark measured a full sort
    // over the whole match set costing several hundred milliseconds, blowing
    // the `<100 ms` budget on its own. Past `RANK_BY_RELEVANCE_LIMIT` matches,
    // `fetch` orders by recency instead, which the list index already
    // answers in the requested order with no sort at all. Recency is a
    // reasonable fallback ranking (see `rank_score`'s own recency term) and
    // ordinary queries, which match far fewer messages, are unaffected.
    let rank_by_relevance = request.order == postio_search::ResultOrder::Relevance
        && plan.has_match
        && total_hits <= RANK_BY_RELEVANCE_LIMIT;
    // The wider pool only pays for itself when ranking by relevance: it is
    // there so a message that scores a little worse on `bm25` but a lot
    // better on recency/affinity can still surface from further down the SQL
    // ordering. Recency-ordered fetches don't need that margin — the SQL
    // order already mostly agrees with `rank_score` — and hydrating a
    // smaller pool matters on the recency path precisely because it is the
    // one a very broad, unrankable match takes.
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
            // Why the order is what it is, in numbers and ids only: the
            // text match, the age, how often the sender was seen, and what
            // they came to.
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
        // Asked for date order, given date order: the fetch already came
        // back `received_at DESC`, and running the ranker over it — even
        // with the scores zeroed — would let sender affinity quietly
        // reorder what the control promises is a plain sort.
        postio_search::ResultOrder::Newest => {}
    }
    candidates.truncate(request.limit as usize);

    let hits: Vec<_> = candidates.into_iter().map(Candidate::into_hit).collect();
    let elapsed = start.elapsed();

    // Shape and cost, never the query. What someone searches their own mail
    // for is as revealing as the mail — a name, an address, an illness — so
    // the *text* is the one thing this line must not carry, however useful it
    // would be. The counts and the timing are what the <100ms budget is
    // argued from anyway.
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
        // Only when nothing matched. A query that worked is not one to
        // second-guess, and this is the one moment the cost is free: there
        // are no rows to draw.
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

/// The term to offer instead, when a query matched nothing.
///
/// # Why only a single bare word
///
/// With two terms, correcting one is a guess about which of them was wrong,
/// and the wrong guess reads as the app misunderstanding the question. With a
/// filter in the query — `from:ada hanah` — the filter is the likelier reason
/// nothing matched. Both cases are left alone rather than answered badly.
///
/// # Where the candidates come from
///
/// From the index's own term dictionary, through the fork of the engine this
/// workspace builds on (the `[patch]` in the root `Cargo.toml`): an unquoted
/// `word~N` is expanded to every term that begins within `N` edits of the
/// word, and `word*` to every term that begins with it —
/// [`postio_search::suggest::widened`] builds the string. So what is read is
/// the handful of messages holding a word *like* the one typed, from every
/// message in the store, bodies included; and the word the offer names is
/// recovered from their text, because the index answers with rows, not with
/// the terms it expanded to.
///
/// This replaced a vocabulary rebuilt from the newest 5,000 senders and
/// subjects: a sample, so a list whose mail was older than that was never
/// offered, and a word only a body held never was either. The widened query
/// runs here, on a search that found nothing, and in [`near_words`], for the
/// command bar's forgiving search -- never for a rule's query, which stays
/// exact, ADR 0037's whole point.
///
/// Documents are counted within what was read, which is at most
/// [`SUGGESTION_DOCUMENTS`] of each half: enough to rank candidates against
/// each other, and the count the offer shows is then "at least".
async fn suggestion_for(
    connection: &Connection,
    query: &postio_search::ParsedQuery,
) -> Result<Option<postio_search::suggest::Suggestion>> {
    let mut terms = query.searchable_terms();
    let Some(term) = terms.next() else {
        return Ok(None);
    };
    // A quoted word asked for itself, exactly -- see `TextTerm::quoted`.
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

/// The words the index holds that are near `typed`, each with how many of
/// the documents read hold it -- the vocabulary [`suggestion_for`] ranks an
/// offer from and [`near_words`] a forgiving search's words. Empty when
/// `typed` cannot be widened at all.
///
/// The index expands `typed~N` or `typed*` against its own dictionary (see
/// [`suggestion_for`]), and since it answers with rows rather than with the
/// terms it expanded to, the words are recovered from the text of at most
/// [`SUGGESTION_DOCUMENTS`] documents of each half.
async fn words_near(
    connection: &Connection,
    typed: &str,
    beside: &[&str],
) -> Result<std::collections::HashMap<String, u64>> {
    use std::collections::{HashMap, HashSet};

    let Some(widened) = postio_search::suggest::widened(typed) else {
        return Ok(HashMap::new());
    };
    // The words typed beside it, bare, so the index reads the whole thing as
    // one query of bare words -- the only kind it expands `~N` in -- and the
    // documents read are the ones that best match all of it. Without them,
    // "trop" read fifty of the thousand messages saying "trip" and never
    // reached the one girl scout troop.
    let beside: Vec<String> = beside
        .iter()
        .map(|word| word.to_lowercase())
        .filter(|word| !word.is_empty() && word.chars().all(char::is_alphanumeric))
        .collect();
    let metadata_query = std::iter::once(widened.clone())
        .chain(beside.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    // The body column is folded on the way in, so its query is folded the
    // same way — the rule every body query here keeps (ADR 0038).
    let body_query =
        postio_search::suggest::widened(&postio_model::fold::fold(typed)).map(|widened| {
            std::iter::once(widened)
                .chain(beside.iter().map(|word| postio_model::fold::fold(word)))
                .collect::<Vec<_>>()
                .join(" ")
        });

    // Best first, so a common neighbour cannot crowd the meant word out of
    // what is read. `fts_score` is projected bare and with the match's own
    // parameter, or it answers 0.0 (see `HITS_JOIN`).
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

    // Each document counts a word once, however often it repeats it. Split
    // the way the index's tokenizer splits, and lowercased the way it folds.
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

/// The words each unquoted, positive word of a forgiving search is read as
/// ([`postio_search::suggest::near`]), keyed by the word as typed. A word
/// with nothing near it beyond itself is left out, and so is one that
/// cannot be widened: it is searched for as typed.
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
        // Whether the word is one the mailbox holds decides whether it is
        // corrected (`suggest::near`), so it is asked, not left to whether
        // the documents read happened to say it.
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

/// Whether any message holds `word` exactly, in either index.
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

/// How many words one typed word is read as in a forgiving search.
///
/// Enough for a plural, a completion and a misspelling or two; each is one
/// more term in the index's disjunction, so not the whole neighbourhood.
const NEAR_WORDS: usize = 8;

/// Whether every message in the searched scope has a body to search.
///
/// # Why this is not a count, and why it is cheap
///
/// #352 asked for a count of what is missing. Under ADR 0016 that number is a
/// draining queue — every folder backfills to completion by default — so a
/// figure would be alarming about something that needs no action and will be
/// zero on its own. What the surface needs is the boolean.
///
/// Which is also the only version that fits the `<100 ms` budget, and it is
/// read off the folders (#1612). Each mailbox keeps `bodies_owed`, a count
/// the schema's triggers maintain beside its other counts, and the scope is
/// always a set of mailboxes, so the answer is a walk of the account's few
/// folders. It used to be a query over `messages`: first through a partial
/// index this engine will not read
/// (`docs/notes/2026-09-12-a-partial-index-the-planner-will-not-read.md`),
/// then, with that index gone, a walk of every message in the account on a
/// store whose backfill had finished -- the steady state under ADR 0016 --
/// reading each row for `body_state`. One statement, one row, so no count
/// could see it; `whether_the_corpus_is_complete_is_read_off_the_folders_not_the_messages`
/// asks the planner instead.
///
/// Scoped, deliberately: the claim on screen is about the search that was
/// just run, so "complete" has to mean complete *here* — a fully backfilled
/// inbox must not carry a caveat earned by an archive nobody searched.
///
/// # What it asks, and the one case it does not cover
///
/// It asks *does backfill still owe bodies here*, not *is every local body in
/// the index*. Those differ for a message whose body has arrived but whose
/// text has not been indexed yet — the window `postio_session::spawn_body_indexer`
/// closes at startup for a store that predates #327.
///
/// Answering the second question exactly would mean `NOT EXISTS (SELECT 1
/// FROM message_bodies_fts WHERE rowid = m.id)`, which has no partial index
/// behind it: proving *absence* would scan every message in scope, and it
/// would do so precisely when the corpus is complete, which is the common
/// case. That is the full-corpus scan per keystroke the budget forbids.
///
/// So this takes the dominant and durable term. Undownloaded bodies are the
/// overwhelming majority of what search cannot see, and they persist for as
/// long as backfill runs; an unindexed local body is a brief startup state
/// that heals itself. Under-reporting the caveat for a few seconds after
/// launch is the right way to be wrong here — the alternative is a caveat
/// that costs the query its budget forever.
async fn corpus_complete(connection: &Connection, request: &SearchRequest<'_>) -> Result<bool> {
    let (sql, params) = corpus_complete_sql(request);
    let complete = sql::one(connection, &sql, params, |row| row.col(0)).await?;
    Ok(complete)
}

/// The statement [`corpus_complete`] runs, and its parameters, so a test can
/// ask the planner what it reads.
///
/// Read off the folders, not the messages (#1612): each mailbox keeps
/// `bodies_owed`, the count of its visible messages still waiting for a
/// body, and the scope here is always a set of mailboxes -- an account's,
/// narrowed by role. So the answer is one short walk of the account's
/// folders, where it used to be a walk of every message in the account on
/// a store whose backfill had finished, reading each row for `body_state`.
#[doc(hidden)]
pub fn corpus_complete_sql(request: &SearchRequest<'_>) -> (String, Vec<turso::Value>) {
    let mut conditions = vec!["bodies_owed > 0".to_string()];
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

/// The size `larger:` is offered at, when a result set has anything that big.
///
/// The canvas' own `larger:1M`. One threshold rather than a ramp: the chip is
/// a shortcut for "the ones with something heavy attached", and a second size
/// chip would be two ways of saying the same thing.
const LARGE_BYTES: u64 = 1024 * 1024;

/// The `larger:` token [`LARGE_BYTES`] is spelled as. Round-trips through
/// [`postio_search::parse`] — a test asserts it.
const LARGE_TOKEN: &str = "larger:1M";

/// How many folders the refine column offers as `in:` chips.
///
/// Two, so a mailbox that files list traffic into a dozen folders does not
/// spend the whole shortlist on them and crowd out `is:unread`.
const REFINE_FOLDERS: usize = 2;

/// How many documents of each half a suggestion reads.
///
/// The index has already narrowed to messages holding a word like the one
/// typed, so this bounds only how common a candidate can be *shown* to be —
/// ranking needs relative counts, not totals. And it runs only on a search
/// that found nothing.
const SUGGESTION_DOCUMENTS: i64 = 50;

/// Measures what the query's result set is made of: how it splits across the
/// scopes, and which narrowings are worth offering.
///
/// Separate from [`search`] rather than folded into it because they are asked
/// for at different rates. The readout and the result rows follow every
/// keystroke; the facets only need to be right by the time the eye reaches
/// the column beside them, and making every keystroke pay for four extra
/// aggregate queries would spend the interaction budget on a number nobody is
/// looking at yet.
///
/// Every count here is bounded the same way [`SearchResults::total_hits`] is
/// — see [`TOTAL_HITS_CAP`] — so a query broad enough to match a whole
/// mailbox costs the same as any other.
pub async fn facets(connection: &Connection, request: &SearchRequest<'_>) -> Result<Facets> {
    // Scope counts hold the query and vary the scope: the column says what
    // *switching* would find, so it cannot be measured inside the scope the
    // user is already in.
    //
    // Refinements are the opposite: they narrow what is on screen, so they
    // are measured inside the current scope -- and the current scope's own
    // count is the same capped walk they are, so all three come out of one
    // statement (#1612). Only the scopes the user is not in walk the match
    // again; one walk shared across every scope would let a broad match in
    // a large folder fill the cap and read another scope as empty.
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

/// What one walk of the current scope's match yields: its count, and the
/// refinements measured over it. See [`Plan::current_scope`].
struct CurrentScope {
    hits: u64,
    refinements: Vec<Refinement>,
}

/// The pure ranking function: `bm25` (lower is better) adjusted downward by
/// recency and sender affinity, so a better-boosted candidate sorts earlier
/// in the same ascending order `bm25` alone would use.
///
/// Both boosts are bounded in `[0, 1)` before weighting, so a genuinely
/// stronger text match (a much more negative `bm25`) is never overridden by
/// recency or affinity alone — they break ties and nudge close calls, they do
/// not override relevance.
pub fn rank_score(
    bm25: f64,
    received_at: DateTime<Utc>,
    now: DateTime<Utc>,
    sender_times_seen: i64,
) -> f64 {
    let age_days = (now - received_at).num_milliseconds() as f64 / 86_400_000.0;
    let age_days = age_days.max(0.0);
    let recency = (-age_days / RECENCY_HALF_LIFE_DAYS * std::f64::consts::LN_2).exp();

    // log1p rather than the raw count: the difference between a sender seen
    // once and one seen five times should matter more than the difference
    // between 500 and 504.
    let affinity = (1.0 + sender_times_seen.max(0) as f64).ln() / (1.0 + 100f64).ln();
    let affinity = affinity.min(1.0);

    bm25 - RECENCY_WEIGHT * recency - SENDER_WEIGHT * affinity
}

/// How much a body match counts beside a metadata one.
///
/// The decision #408 asked to be written down. Free text used to be one
/// `bm25()` over six columns, and FTS5's own length normalisation did the
/// discriminating for us: a term in a short `subject` scored far better than
/// the same term in a long `body`, which is why a search for "invoice" put
/// the message *about* invoices above the one that mentions them. Two indexes
/// means two scores with no shared corpus statistics, so that has to be said
/// explicitly instead.
///
/// The rule is **the weighted sum of both**, body at half. Summing rather
/// than taking the better of the two, because a message matching in *both* is
/// the most relevant thing there is and should rank first. Half rather than a
/// fitted constant, because the two scores are not on a common scale and
/// pretending to a second decimal place would be false precision — what is
/// being encoded is only the ordering "subject beats body, both beats
/// either", and `a_subject_match_outranks_a_body_match` is the test that
/// holds it.
///
/// bm25 is negative-is-better, so both terms pull the score down and a
/// missing side contributes zero.
const BODY_SCORE_WEIGHT: f64 = 0.5;

/// The two free-text matches, unioned.
///
/// `UNION ALL` of two `MATCH`es rather than an `OR` of them: FTS5 needs
/// `MATCH` as an index constraint, so it cannot be one side of a disjunction
/// in a `WHERE`. Each index is matched in its own arm, where its `bm25()` is
/// meaningful because that cursor is the one FTS5 matched.
///
/// **No `GROUP BY` here, deliberately.** Folding a message that matched in
/// both into one row is the obvious thing to write and it costs the whole
/// query: an aggregate forces SQLite to materialise every match before
/// anything else runs, so a word in most of the mailbox built a 120,000-row
/// temp table to answer a `LIMIT 50` — 297 ms against a 100 ms budget,
/// measured. Without it the subquery is a co-routine and the rows stream.
///
/// The cost is that such a message appears twice, which each caller settles
/// in the way that is cheap for it: the candidate pool de-duplicates in Rust
/// as it reads, the counting queries say `DISTINCT`, and `hydrate` — which
/// sees at most a pool's worth of ids — is the one place that aggregates, and
/// therefore the one place the combined score is computed.
/// The matches are written **first**, and pinned there. Left to itself SQLite
/// cannot estimate a co-routine's size, so it drove from `messages` and
/// probed the matches — walking every message in the account to answer a
/// query that matched 1% of it, measured at 49 ms where the single-index
/// version took 2.9. Driving from the matches is a point lookup per hit into
/// `messages`' own primary key.
///
/// # `fts_score` is projected bare, and negated outside
///
/// This was `bm25()`, where a *more negative* number is a better match, and
/// [`rank_score`] and the sort after it both take that convention: candidates
/// are sorted ascending and the best one is first. `fts_score` is the other
/// way round -- higher is better, which is why the engine's own examples say
/// `ORDER BY score DESC`. So the value has to be negated somewhere.
///
/// **Not here.** `fts_score` answers with a score only when the call is the
/// whole select-list expression; put it inside *any* arithmetic and it
/// answers `0.0`:
///
/// ```text
/// SELECT id, fts_score(subject, ?1)        3.82, 1.87
/// SELECT id, fts_score(subject, ?1) AS s   3.82, 1.87
/// SELECT id, -fts_score(subject, ?1)       0.00, 0.00
/// SELECT id, 0 - fts_score(subject, ?1)    0.00, 0.00
/// ```
///
/// Which is the worst kind of trap: every row still comes back, in the right
/// set, and only the *ranking* is silently gone. With `-fts_score(...)` here,
/// every candidate scored `0.0`, `rank_score` reduced to recency and affinity,
/// and search answered every query in date order. One test noticed
/// (`newest_order_answers_in_date_order_however_the_ranking_disagrees`);
/// nothing else could have.
///
/// So the projection is bare and the outer `SELECT` negates the alias, which
/// is an ordinary column by then. `the_score_is_lost_to_any_arithmetic_around_it`
/// in `postio-storage`'s capability suite is what will notice if a later
/// release makes the wrapped form work.
///
/// # `?1` and `?2`, not four bare `?`s
///
/// `fts_score` also answers `0.0` unless its query term is the *same
/// parameter* as the `fts_match` that selected the row -- not the same value,
/// the same expression. So the term is written once per index and reused. A
/// bare `?` after an explicit `?N` continues from `N + 1` here exactly as it
/// does in SQLite, so the conditions that follow still number themselves.
/// # `?1` and `?2`, not four bare `?`s
///
/// **`fts_score` answers `0.0` unless its query term is the *same parameter*
/// as the `fts_match` that selected the row.** Not the same value — the same
/// expression. Measured:
///
/// ```text
/// fts_match(body, 'report')  fts_score(body, 'report')   3.82, 1.87
/// fts_match(body, ?1)        fts_score(body, ?1)         3.82, 1.87
/// fts_match(body, ?2)        fts_score(body, ?1)         0.00, 0.00
/// fts_match(body, ?1)        fts_score(body, 'report')   0.00, 0.00
/// ```
///
/// Which is a trap rather than an inconvenience: every row still comes back,
/// in the right set, and only the *ranking* is silently gone. With four bare
/// `?`s this statement bound four parameters of equal value and scored every
/// candidate `0.0`, so `rank_score` reduced to recency and affinity and
/// search answered every query in date order. One test noticed
/// (`newest_order_answers_in_date_order_however_the_ranking_disagrees`);
/// nothing else could have.
///
/// So the term is written once per index and reused. A bare `?` after an
/// explicit `?N` continues from `N + 1` here exactly as it does in SQLite, so
/// the conditions that follow still number themselves — and `match_params`
/// binds two values rather than four, in the same order.
///
/// # Why the lookup names its index
///
/// Each hit is a content id, and its messages are found through
/// `idx_messages_content`. That has to be written down, because the planner
/// has no statistics (never `ANALYZE`; `docs/gotchas.md`) and ranks a seek by
/// how many key columns it binds. A `WHERE` saying `m.received_at >= ?`
/// beside `m.account_id = ?` offers it `idx_messages_account_list` with two,
/// against `content_id=?`'s one -- so it took the date range, and walked every
/// message in it once per hit. A word plus `after:` went from under a
/// millisecond to over a second on 20k messages (#1809). Before content
/// identity the lookup was `m.id = hits.rid`, a rowid seek no index could
/// outbid, which is why this was never needed. The sibling check in
/// [`Plan::where_sql`] and the facet join in [`Plan::current_scope`] name it
/// for the same reason; `driven_join_plan` in the index suite holds all three.
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

/// [`HITS_JOIN`] with a third arm: what attachments say (spec 010 D11).
///
/// The conversation search's own, and only its: GTK's [`search`] keeps
/// [`HITS_JOIN`]'s corpus until Linux adopts this search, so the two are
/// separate strings. **Keep the first two arms and the join in step with
/// [`HITS_JOIN`]**: a change to how a hit is joined to its messages there
/// is owed here too.
///
/// The arm is grouped by content: a spreadsheet whose every row says the
/// word is one hit for its message, not thousands, so the walk's `LIMIT`
/// counts messages as it does for the other two. Its score is the best
/// unit's, read bare inside and aggregated outside, for the reason
/// [`HITS_JOIN`] gives about arithmetic around `fts_score`. The attachment
/// text is folded as the body is, so it is matched by the body's `?2`.
///
/// The join names `idx_messages_content` for the reason [`HITS_JOIN`] gives
/// (#1809), and so does every other statement here that looks messages up
/// by content: the set path's arms in `conversations.rs`, a negated word's
/// set ([`NEGATED_WORD_SET`]), a column's set (`fts_column_set`), the Files
/// tab's passages. `driven_join_plan` in the index suite holds them.
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

/// The same match, asked one message at a time.
///
/// Two plans, because two statements want opposite things. [`HITS_JOIN`] is
/// driven *by* the match, which is what a query narrow enough to rank wants:
/// walk the postings, look each hit up by primary key. This is driven by
/// `messages` instead, and is what a query too broad to rank wants — ordered
/// by `m.received_at` off that table's own index, stopping at `LIMIT` after a
/// handful of rows, asking each one "did you match?".
///
/// The asking has to be a *docid seek*, and this shape is what makes it one:
/// `rowid = m.id AND ... MATCH ?` gives FTS5 both constraints, and the plan
/// says `VIRTUAL TABLE INDEX 0:=M5` — the `=` being the rowid. The obvious
/// alternatives are both materialisations: probing [`HITS_JOIN`]'s union
/// per row forces the co-routine into a temp table with an automatic index,
/// and `m.id IN (SELECT rowid ...)` builds an ephemeral b-tree of every
/// posting. On a word in most of the mailbox either is ~120 ms of setup to
/// answer a `LIMIT 50`.
const CORRELATED_MATCH: &str = "(EXISTS (SELECT 1 FROM search_documents d
               WHERE d.content_id = m.content_id
                 AND fts_match(d.sender, d.recipients, d.subject, d.filenames, d.list_id, ?))
   OR EXISTS (SELECT 1 FROM message_search_bodies b
               WHERE b.content_id = m.content_id AND fts_match(b.body_search, ?)))";

/// Which plan a statement asks for. See [`HITS_JOIN`] and [`CORRELATED_MATCH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// Driven by the match: walk the postings, look each hit up.
    Driven,
    /// Driven by `messages`: walk the rows, ask each whether it matched.
    Probed,
}

/// One row pulled out of SQL before ranking.
#[derive(Clone)]
struct Candidate {
    message_id: MessageId,
    thread_id: Option<ThreadId>,
    mailbox_id: MailboxId,
    subject: Option<String>,
    from_name: Option<String>,
    from_address: Option<String>,
    received_at: DateTime<Utc>,
    /// What the ranking ages it from: see [`AGED_FROM`].
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

/// The account-scoped filter every search carries, plus the free-text `MATCH`
/// state, compiled once and shared by the count query and the fetch query.
struct Plan {
    conditions: Vec<String>,
    params: Vec<turso::Value>,
    /// Which accounts the request was about.
    ///
    /// Carried rather than recovered from `params`, which `hydrate` used to
    /// do by reading `params[0]` on the promise that the account id was
    /// always bound first. Under `AccountScope::Unified` there is no account
    /// parameter at all, so that promise became a panic (#186) — and it was
    /// only ever true by the order two `push`es happened to be written in.
    account: AccountScope,
    /// Whether a positive free-text `MATCH` is part of `conditions`, in which
    /// case `messages_fts` must be joined so `bm25()`/`snippet()` can read it.
    has_match: bool,
    /// Whether the words were widened to the words near them: then the
    /// probed shape is never taken, since it would run the disjunction once
    /// per message walked rather than once.
    widened: bool,
    /// The free-text `MATCH` expression itself, when `has_match` is set.
    ///
    /// Kept separately rather than found by position in `params`: a filter
    /// clause (`from:`, `subject:`, ...) also binds its own `messages_fts
    /// MATCH ?` parameter (see `fts_column_condition`) and can appear after
    /// this one, so "the match parameter" is not reliably "the last
    /// parameter" once a query composes free text with an operator —
    /// `hydrate` needs the *free-text* expression specifically, to compute
    /// `bm25`/`snippet` against what the user actually typed as text rather
    /// than, say, an unrelated `from:` value that happens to also be a valid
    /// (if redundant) constraint on the same rows.
    match_param: Option<turso::Value>,
    /// The same expression for the body index: each term folded the way the
    /// body text was, joined after folding. See [`Plan::match_params`].
    body_match_param: Option<turso::Value>,
    /// The filters taken out of `conditions` as sets of message ids, when
    /// the plan was built by [`Plan::build_sets`]; empty otherwise.
    sets: Vec<IdSet>,
}

/// One filter as the set of messages it keeps (or, negated, refuses): the
/// subquery that names them, and its parameters.
///
/// A conversation search's way round a plan this engine makes of
/// `m.id IN (subquery)`: tested once per row walked, at a cost that grows
/// with the subquery's size, so `budget from:ada` over a common word walked
/// every message the word matched and compared it against every message Ada
/// sent -- 375 ms on 20,000 messages, measured. Read once, as a set, the
/// same filter costs about a millisecond.
#[derive(Debug, Clone)]
struct IdSet {
    sql: String,
    params: Vec<turso::Value>,
    negated: bool,
}

/// What a negated word refuses: the messages either index says carry it.
/// The indexes are keyed by content, so each content is joined to every
/// message that carries it, as [`HITS_JOIN`] joins its hits.
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

    /// [`Plan::build`], with every filter that is a set of messages
    /// ([`id_set`]) and every negated word taken out of `conditions` and
    /// into [`Plan::sets`], for a caller that reads them once and applies
    /// them itself. [`search`] never does: its plan is unchanged.
    fn build_sets(request: &SearchRequest<'_>) -> Self {
        Self::build_with(request, &Default::default(), true)
    }

    /// The plan with each word `near` names read as any of its words: the
    /// forgiving search's second pass. Every word is still a quoted literal,
    /// so the index reads the disjunction exactly as written.
    fn build_near(
        request: &SearchRequest<'_>,
        near: &std::collections::HashMap<String, Vec<String>>,
    ) -> Self {
        Self::build_with(request, near, false)
    }

    fn build_with(
        request: &SearchRequest<'_>,
        near: &std::collections::HashMap<String, Vec<String>>,
        as_sets: bool,
    ) -> Self {
        // `AccountScope::Unified` names no account, so the predicate is
        // absent rather than widened -- which is why migration 0012 exists:
        // without `idx_messages_recency` the recency path has no index that
        // can supply its ordering once this conjunct is gone (ADR 0005 Q5a).
        let mut conditions = vec!["m.deleted_locally = 0".to_string()];
        let mut params: Vec<turso::Value> = Vec::new();
        match request.account.account() {
            Some(id) => {
                conditions.push("m.account_id = ?".to_string());
                params.push(turso::Value::Integer(id.get()));
            }
            // `Unified` is "every **enabled** account", not "every account",
            // and the difference only became observable when #961 gave the
            // application a way to construct this scope. Dropping the account
            // conjunct and stopping there reaches mail from an account the
            // user has switched off, and from one that is midway through
            // being deleted -- which shows up as somebody else's mail in a
            // result list with no row saying where it came from (ADR 0005
            // Q10).
            //
            // The same predicate `AccountRepository::list_enabled` uses, and
            // deliberately a subquery rather than a join: `accounts` holds a
            // handful of rows, the outer query is already driven by the index
            // or by `hits`, and a join would give the planner a second table
            // to choose an order for on the path the 100 ms budget is
            // measured against.
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

        // Negated terms are excluded across both indexes rather than folded
        // into each one's own match, and that is a correctness fix rather
        // than tidiness. `("report") NOT ("spam")` asked of the metadata
        // index alone is true for a message whose "spam" is in its *body* --
        // the metadata genuinely does not contain it -- so the message came
        // back from a query that had explicitly refused it. An exclusion has
        // to be about the message, and only a condition outside the join can
        // be.
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
            // The body half folded, the metadata half not -- the same rule
            // `match_params` keeps, for the same reason.
            params.push(turso::Value::Text(literal.clone()));
            params.push(turso::Value::Text(postio_model::fold::fold(&literal)));
        }

        // Each positive word, as the words it may be read as: itself, or in
        // a forgiving search's second pass the words near it.
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
        // `("ticket" OR "tickets") AND "southwest"`: a disjunction of
        // literals rather than the index's own `ticket~1`, which it expands
        // only in a query of bare words and reads as plain text beside a
        // quoted one or an AND. The body half folds each word, never the
        // whole expression: folded, `OR` is the word "or", which nearly
        // every message says.
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
            // The match itself has moved into the join (see `Plan::join_sql`),
            // because free text now has to reach two indexes and a row that
            // matched in either one is a hit. `MATCH` cannot be written as an
            // `OR` of two tables in a `WHERE` -- FTS5 needs it as an index
            // constraint -- so each is matched in its own subquery and the
            // join is what unions them.
            //
            // Nothing is pushed onto `params` here: the join's parameters sit
            // before every condition's in the statement text, and
            // `from_params` is what binds them.
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

        for clause in request.query.filters() {
            if as_sets && let Some((sql, params)) = id_set(&clause.filter) {
                sets.push(IdSet {
                    sql,
                    params,
                    negated: clause.negated,
                });
                continue;
            }
            let (sql, mut values) = filter_condition(&clause.filter);
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

    /// How one statement reaches the free-text match.
    ///
    /// Not a tuning knob: the two forms have different *plans*, and which one
    /// is right depends on what the statement is about to do with the rows.
    /// See [`HITS_JOIN`] and [`CORRELATED_MATCH`].
    fn where_sql(&self, form: Form) -> String {
        let eligible = self.occurrence_where_sql(form);
        // A sibling must satisfy the same location predicates before it can
        // represent this content. Reusing numbered parameters keeps this a
        // bounded indexed sibling seek, including on the recency-driven path.
        // Replace the occurrence alias, not the suffix of another alias
        // such as contact-group membership's `gm`.
        let mut earlier = String::new();
        let mut cursor = 0;
        for (offset, _) in eligible.match_indices("m.") {
            if offset > 0
                && (eligible.as_bytes()[offset - 1].is_ascii_alphanumeric()
                    || eligible.as_bytes()[offset - 1] == b'_')
            {
                continue;
            }
            earlier.push_str(&eligible[cursor..offset]);
            earlier.push_str("earlier.");
            cursor = offset + 2;
        }
        earlier.push_str(&eligible[cursor..]);
        let mut conditions = vec![
            eligible,
            format!(
                "NOT EXISTS (SELECT 1 FROM messages earlier INDEXED BY idx_messages_content \
             WHERE earlier.content_id = m.content_id AND earlier.id < m.id \
               AND ({earlier}))"
            ),
        ];
        if self.has_match {
            conditions.push(
                match form {
                    Form::Driven => "hits.rid IS NOT NULL",
                    Form::Probed => CORRELATED_MATCH,
                }
                .to_string(),
            );
        }
        conditions.join(" AND ")
    }

    /// Location predicates, with reusable bindings for selecting siblings and
    /// measuring facets. Content matches are identical across memberships.
    fn occurrence_where_sql(&self, form: Form) -> String {
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
        sql
    }

    /// Every parameter one statement binds, in the order its `?`s appear.
    ///
    /// The form decides that order, which is the whole reason this is not a
    /// field: a driven statement matches in its `FROM`, so its two
    /// expressions come first, and a probed one matches in its `WHERE`, so
    /// they come last.
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

    /// The expressions the free-text match binds, one per index.
    ///
    /// They come **before** every condition's, because the join is written
    /// before the `WHERE` and SQLite binds `?` left to right. Kept as its own
    /// list rather than pushed onto `params` in `build`, so that every caller
    /// composing a statement has to think about the order once, here, rather
    /// than each getting it right separately.
    fn match_params(&self, _form: Form) -> Vec<turso::Value> {
        let (Some(expr), Some(folded)) = (&self.match_param, &self.body_match_param) else {
            return Vec::new();
        };
        // The body index is built over folded text, so the body's half of the
        // expression is folded to match. The metadata index is not -- its
        // columns are stored as they read -- so that half goes through
        // unchanged. Both or neither, per `postio_model::fold`.
        //
        // Each term is folded before the terms are joined, never the joined
        // expression: folding lowercases, and to the index a lowercase `and`
        // is a word rather than the operator. Folding the whole of
        // `"meeting" AND "agenda"` asked every body for "meeting", "and" or
        // "agenda" -- most of a real mailbox, 200 seconds to count.
        // Two, either way. The driven form writes the term as `?1`/`?2` and
        // uses each twice -- once to score, once to match -- because
        // `fts_score` returns `0.0` when the two are different parameters;
        // see [`HITS_JOIN`]. The probed form has no score and one `fts_match`
        // per arm, and its `?`s are bare because its match sits in the
        // `WHERE`, after the conditions.
        vec![expr.clone(), folded.clone()]
    }

    /// [`Plan::source_sql`], but for `fetch` specifically, where the join order
    /// matters in a way `count` never sees.
    ///
    /// A plain `JOIN` lets SQLite pick which side drives the loop, which is
    /// exactly what `rank_by_relevance` wants: for a match narrow enough to
    /// rank, driving from `messages_fts`'s own `bm25`-ordered scan and
    /// stopping at `LIMIT` is the fast path. But for a match too broad to
    /// rank (see the comment in [`search`]), the query orders by
    /// `m.received_at` instead, and the *same* plain `JOIN` had SQLite
    /// estimate the `MATCH` as selective, drive from `messages_fts` anyway,
    /// and sort the entire match set in a temp b-tree to satisfy that
    /// `ORDER BY` — measured at three quarters of a second on the "common
    /// word" shape of postio-y47's benchmark. `CROSS JOIN` is SQLite's
    /// documented way to pin the join order to how the tables are written
    /// here: `messages` first, driven by its own `(account_id, received_at)`
    /// index, with `messages_fts` tested one row at a time as a cheap
    /// point lookup rather than scanned.
    fn fetch_form(&self, rank_by_relevance: bool, total_hits: u64) -> Form {
        if rank_by_relevance || self.widened || total_hits <= PROBED_FORM_LIMIT {
            Form::Driven
        } else {
            Form::Probed
        }
    }

    /// Counts matches, up to [`TOTAL_HITS_CAP`].
    ///
    /// A term common enough to be in most of a large mailbox forces a
    /// full-postings walk to count exactly — FTS5 gives SQL no cheaper way to
    /// ask "how many". Wrapping the scan in its own `LIMIT` bounds that cost
    /// regardless of how broad the match is, at the price of an exact count
    /// past the cap. See [`SearchResults::total_hits_capped`].
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

    /// The current scope's match, walked once: its count, the flag-shaped
    /// refinements (unread, flagged, attachments, size) and the folders the
    /// matches are in, from one statement grouped by folder (#1612).
    ///
    /// These were two walks and a third count of the same capped match in
    /// the same scope. Grouped by folder, each row carries its folder's hits
    /// and its flag sums: the flags are summed across the rows, the folders
    /// are the rows ranked, and the count is their total. The inner `LIMIT`
    /// is [`TOTAL_HITS_CAP`], the same bound [`Plan::count`] uses, so every
    /// number is a floor past the cap exactly as [`SearchResults::total_hits`]
    /// is -- and since [`Facets::suggested`] only compares them against that
    /// same capped total, a capped set still ranks its refinements correctly.
    ///
    /// The folder chips are spelled `in:` rather than canvas 2b's `list:`
    /// because `list:` cannot yet be answered exactly -- see [`Scope::Lists`]
    /// and `postio-0bz`. `in:` names the same folder and is exact today.
    ///
    /// `LARGE_BYTES` is written into the SQL rather than bound, and that is
    /// not a shortcut: a bound `?` for it would sit before the `{from}` that
    /// [`HITS_JOIN`] numbers `?1` and `?2`, which is how this statement once
    /// bound five parameters into three slots. Nothing user-supplied is
    /// interpolated; every value the caller controls is still a parameter.
    async fn current_scope(&self, connection: &Connection) -> Result<CurrentScope> {
        // Cap content identities first, then measure every qualifying
        // membership. Summing per-folder counts would double-count content;
        // choosing one folder first would hide valid folder refinements.
        let sql = format!(
            "WITH capped AS (
                 SELECT DISTINCT m.content_id {from} WHERE {where_sql} LIMIT ?
             ), matched AS (
                 SELECT m.content_id, mb.name, m.seen, m.flagged, m.has_attachments, m.size
                   FROM capped c CROSS JOIN messages m INDEXED BY idx_messages_content
                     ON m.content_id = c.content_id
                   JOIN mailboxes mb ON mb.id = m.mailbox_id
                  WHERE {eligible}
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

    /// Selects a candidate pool, then hydrates it into full [`Candidate`]s.
    ///
    /// Two queries rather than one: the first selects only `m.id`, ordered
    /// and cut down to `pool_size`, and is the query that has to be fast
    /// across every match size — the plan discussed in
    /// [`Plan::fetch_form`] depends on the query being simple enough for
    /// SQLite to recognize. Folding in the per-row correlated subqueries
    /// (sender name/address, contact affinity, snippet) for *every* matching
    /// row, before the `LIMIT` narrows it, was measured to cost the same
    /// several hundred milliseconds on a broad match that `count` used to —
    /// even though the id-only shape alone was fast, adding those columns
    /// back to the same statement was enough to lose the plan again. Hydrating
    /// afterward, for only the (at most `pool_size`) ids that survive, keeps
    /// that cost paid once per candidate rather than once per match.
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

    /// The candidate pool: an id and the combined `bm25` for each, in the
    /// order SQL chose them.
    ///
    /// The scores come back **here** rather than from `hydrate`, and that is
    /// what keeps this affordable. Hydrating used to re-ask the indexes for
    /// the scores of the ids it was given, which for a word in most of the
    /// mailbox meant walking every posting a second time — 80 ms of a 100 ms
    /// budget, whether the second walk was a `GROUP BY` over the whole union
    /// or an `IN` list FTS5 declines to use as a docid constraint. The pool
    /// query has the scores in hand already; carrying them out costs nothing.
    async fn fetch_candidates(
        &self,
        connection: &Connection,
        pool_size: u32,
        rank_by_relevance: bool,
        total_hits: u64,
        now: DateTime<Utc>,
    ) -> Result<Vec<(i64, f64)>> {
        let form = self.fetch_form(rank_by_relevance, total_hits);
        // The row's own score on the driven path: without a `GROUP BY` each
        // arm of the union contributes its own row, so a message that matched
        // in both is ordered by its better half and summed below. That is a
        // *pool* ordering, not the answer — `search` re-ranks what comes back
        // through `rank_score` — so all it has to get right is which
        // candidates are worth hydrating.
        // The pool ordering, and the age term in it is load-bearing rather
        // than a refinement. Ordered by `bm25` alone, every one of the 400
        // candidates this handed to `rank_score` for `invoice` on a real store
        // was over a year old -- so no ranking function downstream could
        // surface a recent message, because none was ever in the pool.
        //
        // Linear in years because this build's SQLite has no `exp`; see
        // `POOL_AGE_WEIGHT_PER_YEAR`. `?` is bound to now, in milliseconds.
        let order_by = if rank_by_relevance {
            &format!(
                "-coalesce(hits.meta, 0.0) - coalesce(hits.body, 0.0) \
                 + {POOL_AGE_WEIGHT_PER_YEAR} * (? - {AGED_FROM}) / {MILLIS_PER_YEAR}"
            )
        } else {
            "m.received_at DESC"
        };
        // The probed path carries no scores, and that is a decision rather
        // than an omission. It is only ever taken for a match too broad to
        // rank by relevance — `search` says so — where `bm25` across the
        // whole match is near-uniform anyway and recency is the fallback
        // ranking on purpose. Asking for the scores there costs the plan:
        // adding *any* column to this statement was enough to lose it and
        // spend half a second (the same trap `fetch`'s own docs record for
        // the hydrate columns), and correlated subqueries in the select list
        // are no exception.
        let scores = match (self.has_match, form) {
            // Negated here rather than in the projection inside the union:
            // see [`HITS_JOIN`]. By this point they are ordinary columns.
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
        // `now`, for the age term in the pool ordering. Bound here because
        // parameters are positional and the `ORDER BY` sits between the
        // `WHERE`'s and the `LIMIT`'s -- and only when that ordering is the
        // one carrying the term, or the count would not match the statement.
        if rank_by_relevance {
            params.push(turso::Value::Integer(now.timestamp_millis()));
        }
        // Asked for more than the pool, because the union can hand back the
        // same message twice and the duplicates are folded below. Doubling is
        // the bound: a message appears at most once per index.
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

        // Folded here rather than with a `GROUP BY`, which would cost a sort
        // over the match set — the thing this whole shape exists to avoid.
        // Adding the two is what makes "matched in both indexes" the best a
        // message can do; `bm25` is negative-is-better, so each term pulls the
        // score down and a missing side adds nothing.
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

    /// Fetches the full row for each of `ids`, preserving their order.
    ///
    /// `ids` is small (bounded by the candidate pool, at most a few
    /// thousand), so the `IN` list and the per-row correlated subqueries here
    /// are cheap regardless of how many messages the query as a whole
    /// matched.
    /// Fetches the full row for each candidate, preserving the pool's order.
    ///
    /// No FTS at all: the scores arrived with the ids. This is strictly less
    /// work than it was before the body index existed, when it re-computed
    /// `bm25` and cut a `snippet()` here.
    /// The statement `hydrate` runs, with `placeholders` standing in for the
    /// `IN` list. Its own function so a test can hold its query plan still.
    fn hydrate_sql(&self, placeholders: &str) -> String {
        // Sender affinity is per account, or shared when `account_id IS
        // NULL`. Scoped to one account, only that account's sightings count;
        // unified, every account's do — which is the right answer rather than
        // a shortcut, since the question "how often do I hear from this
        // person" is about the person and not about which inbox they landed
        // in.
        let affinity = match self.account.account() {
            Some(_) => "AND (c.account_id = ? OR c.account_id IS NULL)",
            None => "",
        };
        // The contacts probe compares against `sub.from_normalized` — a
        // plain column of the row source — and never against a nested
        // subquery. With the subquery as the comparand, SQLite cannot use it
        // as an index key: the probe pinned only `account_id` and walked
        // every contact of the account once per hydrated candidate,
        // re-evaluating the inner recipients lookup per contact row.
        // `O(candidates × contacts)` — measured at 4.5 s for a 320-match
        // query against 18k contacts, 15 s for a full pool (#746). Hoisted,
        // the same workload is single-digit milliseconds, and
        // `hydrate_probes_contacts_by_address_key` pins the plan.
        // **One correlated lookup, not three.** This asked `recipients` for the
        // sender three times per candidate -- the name, the address, and the
        // normalized address -- and the last two were the same row reached by
        // the same join, differing only in the column selected. Measured on a
        // real store after the search plan was fixed, this statement was
        // 384 ms of a 520 ms cold search: the largest single cost in a search,
        // and two thirds of it was asking the same question again.
        //
        // So the subquery finds the sender's `recipients` row once, and the
        // outer query joins it and `addresses` by primary key. The contacts
        // probe still compares against a plain column -- now the joined
        // `a.address_normalized` rather than a nested subquery -- which is
        // what keeps it on `idx_contacts_account_address` rather than walking
        // every contact per candidate (#746, and
        // `hydrate_probes_contacts_by_address_key` pins it).
        format!(
            "SELECT
                 sub.id, sub.thread_id, sub.mailbox_id, sub.subject, sub.received_at,
                 sender.name AS from_name, a.address AS from_address,
                 (SELECT max(c.times_seen) FROM contacts c
                    WHERE c.address_normalized = a.address_normalized
                      {affinity}) AS sender_times_seen,
                 sub.preview, sub.aged_from
             FROM (SELECT
                     m.id, m.thread_id, m.mailbox_id, m.subject, m.received_at, m.preview,
                     {AGED_FROM} AS aged_from,
                     (SELECT r.id FROM recipients r
                        WHERE r.message_id = m.id AND r.kind = 'from'
                        ORDER BY r.position LIMIT 1) AS from_recipient
                   FROM messages m WHERE m.id IN ({placeholders})) sub
             LEFT JOIN recipients sender ON sender.id = sub.from_recipient
             LEFT JOIN addresses a ON a.id = sender.address_id",
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

        // No `snippet()`. It is an FTS5 function over indexed content and
        // `message_bodies_fts` has none by design (#407), so the excerpt is
        // cut by `postio_search::highlight::snippet` from the body the
        // caller reads out of the blob store. That is a stronger guarantee
        // than this was: the text highlighted is the text that was indexed,
        // rather than SQLite's reconstruction of a separate copy.
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(", ");

        let sql = self.hydrate_sql(&placeholders);

        // Then one parameter per id in the `IN` list, after the affinity
        // subquery's own if it has one.
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
                        // Filled in below, from the pool.
                        bm25: 0.0,
                        // Filled by whoever can read the body — see
                        // `SearchHit::snippet`.
                        snippet: String::new(),
                        score: 0.0,
                    },
                ))
            })
            .await?
            .into_iter()
            .collect();

        // `hydrate`'s own query has no `ORDER BY`; the caller's ordering (by
        // relevance or by recency) lives entirely in the pool's order.
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

/// Translates a [`Scope`] into a SQL condition, or `None` for the scope that
/// constrains nothing.
///
/// Scoped by mailbox *role* rather than by id, because the scope has to mean
/// the same thing on every account and before any folder has been chosen. See
/// [`Scope::Lists`] for why "lists" is a role test and not a `List-Id` one.
fn scope_condition(
    scope: Scope,
    account: AccountScope,
    names_a_folder: bool,
) -> Option<(String, Vec<turso::Value>)> {
    let role = scope_role(scope, names_a_folder)?;
    // The role half is byte-for-byte the same in both scopes. Unified drops
    // the account conjunct and nothing else, which is what makes "every
    // account's inbox" a predicate removal rather than a redefinition of what
    // "Inbox" means (#186, ADR 0005 Q5a).
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

/// Which mailbox roles a [`Scope`] takes in, as a condition on `mailboxes`,
/// or `None` for the scope that constrains nothing. Shared by the search's
/// own scope and by [`corpus_complete_sql`], so the two cannot disagree
/// about which folders a scope means.
fn scope_role(scope: Scope, names_a_folder: bool) -> Option<&'static str> {
    Some(match scope {
        // "All mail" is every folder except drafts, junk and trash
        // (maintainer's decision, #1523): a search is navigation, and what a
        // person is navigating to is almost never a draft of what they were
        // going to say, something they binned, or spam. Sent stays in. An
        // `in:` anywhere in the query lifts the exclusion, because a query
        // that names a folder is already confined to it, and the one thing
        // the exclusion could then do is hide the folder they named.
        Scope::AllMail if names_a_folder => return None,
        Scope::AllMail => "role NOT IN ('drafts', 'junk', 'trash')",
        Scope::Inbox => "role = 'inbox'",
        Scope::Lists => "role = 'regular'",
    })
}

/// Whether the query confines itself to a folder with an `in:` of its own.
///
/// Only an affirmative one: `-in:trash` is a person keeping the default
/// exclusion and adding to it, not asking to see the trash.
fn names_a_folder(query: &ParsedQuery) -> bool {
    // A set of folders names them too (D26).
    query
        .filters()
        .any(|clause| !clause.negated && clause.filter.field() == postio_search::query::Field::In)
}

/// Translates one structured filter into a SQL condition (unnegated) plus its
/// bound parameters, in the order the `?` placeholders appear.
fn filter_condition(filter: &Filter) -> (String, Vec<turso::Value>) {
    match filter {
        Filter::From(value) => fts_column_condition("sender", value),
        Filter::To(value) => fts_column_condition("recipients", value),
        Filter::Subject(value) => fts_column_condition("subject", value),
        // Resolved in SQL rather than in Rust, exactly as `in:` is below: the
        // name the user typed is matched against the account's display name
        // and its address. A name that resolves to nothing yields an empty
        // `IN` set and so matches nothing -- never everything, which is what
        // dropping an unresolvable predicate would silently mean.
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
        // ADR 0007 Q3: "from or to any member", resolved against `recipients`
        // by exact address rather than full text -- a group names people by
        // address, not by whatever words happen to appear near their name.
        // An unresolvable group name is an empty member set and therefore
        // matches nothing, the same "never everything" rule `Account` and
        // `In` follow just above.
        Filter::Group(value) => (
            format!("m.id IN ({GROUP_SET})"),
            vec![turso::Value::Text(value.clone())],
        ),
        // ADR 0025 Q2, and the one operator that is not an FTS `MATCH`. Header
        // values are short and structured -- `spf=pass`, `1.5.24`,
        // `multipart/signed` -- and a tokenizer takes them apart at exactly
        // the characters that made them meaningful, so this is a substring
        // match on a column.
        //
        // `EXISTS` correlated on `m.id`, narrowing on `name` first, which
        // `idx_message_headers_name` answers as a range scan over one name.
        // The name is compared for equality and never as a substring:
        // `header:x-mail` must not find `X-Mailer`, and it is the row shape
        // rather than the SQL that makes a name and a value from *different*
        // fields impossible to pair.
        Filter::Header { name, value } => match value {
            None => (
                "EXISTS (SELECT 1 FROM message_headers h \
                  WHERE h.content_id = m.content_id AND h.name = ?)"
                    .to_string(),
                vec![turso::Value::Text(name.clone())],
            ),
            // `LIKE` folds ASCII case on its own, which is what ADR 0025 Q6
            // asks for. It does not fold anything else, so a value that
            // survived RFC 2047 decoding as non-ASCII matches in the case it
            // was stored -- SQLite's `lower()` is ASCII-only too, so there is
            // nothing to gain by wrapping it.
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
        // `list:` names a mailing list by its `List-Id` (#9): the bracketed
        // identifier `postio-model::mime::list_id_from_text` extracts and
        // `messages.list_id` stores, indexed here the same way `subject` is.
        Filter::List(value) => fts_column_condition("list_id", value),
        Filter::HasAttachment => ("m.has_attachments = 1".to_string(), Vec::new()),
        // Spec 010 (S2): a label by name, in whichever account owns the
        // message -- the join through `message_labels` already keeps it to
        // the message's own account, so the scope needs no clause of its
        // own. Case-insensitive the way the name is unique
        // (`idx_labels_account_name`). A name no label has is an empty
        // `EXISTS` and matches nothing, never everything, as `account:` and
        // `in:` above.
        Filter::Label(value) => (
            "EXISTS (SELECT 1 FROM message_labels ml \
              JOIN labels l ON l.id = ml.label_id \
              WHERE ml.message_id = m.id AND lower(l.name) = lower(?))"
                .to_string(),
            vec![turso::Value::Text(value.clone())],
        ),
        // An open marker: one the person has not dismissed. `markers` is
        // keyed on the message, so this is one primary-key probe a row.
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
                // The promoted headers' columns (spec 007, research R8).
                // NULL is "not known yet", and a NULL comparison is no
                // match: mail whose headers nothing has read is neither.
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
        // Either of several values (spec 010, D26): each member's own
        // condition, ORed. The parameters follow the placeholders, member
        // by member, left to right.
        Filter::AnyOf(set) => {
            let mut sql = Vec::with_capacity(set.members().len());
            let mut params = Vec::new();
            for member in set.members() {
                let (condition, mut values) = filter_condition(member);
                sql.push(format!("({condition})"));
                params.append(&mut values);
            }
            (format!("({})", sql.join(" OR ")), params)
        }
    }
}

/// Builds a condition against one indexed column of `search_documents`.
///
/// This is why `from:`/`to:`/`subject:`/`filename:`/`list:` match whole
/// tokens (as FTS5 tokenizes them) rather than an arbitrary substring: the
/// first version of this filter used `LIKE '%value%'` against `recipients`
/// and `attachments` directly, correlated to the outer message — cheap per
/// row, but `total_hits`'s `count(*)` has no `LIMIT` to short-circuit it, so
/// a plain `from:` search over a large mailbox paid for one such scan per
/// message in the account and blew the `<100 ms` budget (postio-y47's
/// benchmark caught this). Querying the column the index already covers turns
/// that into a single inverted-index lookup, the same cost class as free
/// text.
///
/// # Why the term is asked twice
///
/// FTS5 scoped a match to one column inside the query string —
/// `messages_fts MATCH 'sender:ada'`. This engine takes the columns as
/// arguments instead, and `fts_match(sender, ?)` on its own is *correct* and
/// **does not use the index**: a subset of an index's columns gets `SCAN`,
/// which is exactly the per-message scan the paragraph above is about.
///
/// So the term is asked twice. The five-column form narrows through the
/// index to messages carrying the term *anywhere*; the one-column form then
/// says which column it had to be in. Both are token matches, so
/// `from:`/`to:`/`subject:`/`filename:`/`list:` keep matching whole tokens
/// rather than substrings — and the per-row check only ever runs on what the
/// index already narrowed to.
///
/// Verified in `turso_capabilities.rs`: a column subset alone scans, and the
/// pair uses the index.
fn fts_column_condition(column: &str, value: &str) -> (String, Vec<turso::Value>) {
    let (contents, params) = fts_column_contents(column, value);
    (format!("m.content_id IN ({contents})"), params)
}

/// The messages whose `column` carries `value`: [`fts_column_condition`]'s
/// subquery as a set of message ids. The index is keyed by content, so each
/// content it names is joined to every message that carries it.
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

/// The contents whose `column` carries `value`, asked twice as
/// [`fts_column_condition`] explains.
fn fts_column_contents(column: &str, value: &str) -> (String, Vec<turso::Value>) {
    let literal = fts_literal(value);
    (
        format!(
            "SELECT content_id FROM search_documents
              WHERE fts_match(sender, recipients, subject, filenames, list_id, ?)
                AND fts_match({column}, ?)"
        ),
        vec![
            turso::Value::Text(literal.clone()),
            turso::Value::Text(literal),
        ],
    )
}

/// The messages from or to any member of a group, by name: `group:`'s
/// subquery (ADR 0007 Q3).
const GROUP_SET: &str = "SELECT r.message_id FROM recipients r \
     JOIN addresses a ON a.id = r.address_id \
     WHERE r.kind IN ('from', 'to', 'cc', 'bcc') \
       AND a.address_normalized IN ( \
         SELECT c.address_normalized FROM contact_group_members gm \
         JOIN contacts c ON c.id = gm.contact_id \
         JOIN contact_groups g ON g.id = gm.group_id \
         WHERE lower(g.name) = lower(?))";

/// A filter as the set of messages it keeps, for [`Plan::build_sets`]: the
/// filters that are a lookup of message ids rather than a test of the
/// message's own row. `None` for the rest, which stay conditions.
///
/// `label:` and `has:action` are `EXISTS` probes in [`filter_condition`],
/// cheap per row; as sets they are one walk of `message_labels` or
/// `markers`, which lets a conversation search walk only the messages they
/// keep when nothing else narrows it.
fn id_set(filter: &Filter) -> Option<(String, Vec<turso::Value>)> {
    Some(match filter {
        // A set (D26) is the union of its members' sets, when each is one;
        // `UNION` rather than `UNION ALL`, because a set may also drive the
        // walk (`relaxations`), which must see each message once.
        Filter::AnyOf(set) => {
            let mut arms = Vec::with_capacity(set.members().len());
            let mut params = Vec::new();
            for member in set.members() {
                let (sql, mut values) = id_set(member)?;
                arms.push(format!("SELECT message_id FROM ({sql})"));
                params.append(&mut values);
            }
            (arms.join(" UNION "), params)
        }
        Filter::From(value) => fts_column_set("sender", value),
        Filter::To(value) => fts_column_set("recipients", value),
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

/// A literal for the middle of a `LIKE '%' || ? || '%' ESCAPE '\\'` pattern.
///
/// `%` and `_` are wildcards to `LIKE`, and a header value is exactly where
/// they turn up in ordinary text: `Content-Type: multipart/signed;
/// boundary=__part_1`, a spam score of `100%`. Without this,
/// `header:x-spam-status=100%` would match anything with an `X-Spam-Status`
/// at all -- a wrong answer that looks like a right one.
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

/// Quotes an operator value that would otherwise not survive being typed
/// back in — a folder called `Old Mail` has to become `in:"Old Mail"` or the
/// parser reads it as `in:Old` and a stray word.
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

    /// A plan with a free-text match, which is the only case the form matters
    /// for -- without one there is nothing to drive the join from.
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
        // Three invoices from one template: the text differs by a few
        // percent, which is noise to a reader, so they are one band and
        // carry the best one's score.
        assert_eq!(
            text_bands(&[-18.19, -17.61, -17.19, -6.9, -6.5, -2.0]),
            vec![-18.19, -18.19, -18.19, -6.9, -6.9, -2.0]
        );
    }

    #[test]
    fn a_band_is_measured_from_its_best_so_it_cannot_drift() {
        // Each a little worse than the one before, but the fourth is more
        // than the tolerance below the first: a new band, not a chain.
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
        // The complaint this answers: search surfacing very old mail. On a real
        // store the forty best `bm25` matches for a term had a median age of
        // 6,687 days, and the recency term was zero for every one of them --
        // an exponential with a fourteen-day half-life is zero to four decimal
        // places by then, so `bm25` alone decided the order.
        //
        // What matters is not that recency is *large* but that it still
        // *differs* between candidates. A term that gives every one the same
        // number cannot reorder anything, however heavily weighted.
        let now = at(0);
        let five_years = rank_score(-6.0, at(1825), now, 0);
        let eighteen_years = rank_score(-6.0, at(6570), now, 0);
        assert!(
            five_years < eighteen_years,
            "five-year-old mail must outrank eighteen-year-old at equal bm25: \
             {five_years} against {eighteen_years}"
        );

        // And the gap has to be big enough to matter against the spread real
        // candidates sit in -- 0.80 bm25 points across the top forty on that
        // store. A separation smaller than that reorders nothing in practice.
        assert!(
            eighteen_years - five_years > 0.40,
            "the gap between five and eighteen years is {}, too small to move \
             anything against a bm25 spread of 0.80",
            eighteen_years - five_years
        );
    }

    #[test]
    fn a_much_better_match_still_beats_a_more_recent_one() {
        // Recency is weighted heavily on purpose, but it is still a boost and
        // not an override: `RECENCY_WEIGHT` bounds what it can move, so a
        // genuinely far stronger text match wins.
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

        // Narrow enough to rank: driven by the match, ordered by bm25.
        assert_eq!(plan.fetch_form(true, 500), Form::Driven);

        // Too broad to rank, so it orders by recency -- and *used to* switch
        // the join form on the same threshold. That is the bug: ordering and
        // join form are separate questions. Driving from `messages` and asking
        // each row "did you match?" costs whatever it takes to walk back to
        // the matches, so a term scattered through old mail scanned deep.
        // Measured on a real store, `invoice` at 3,843 hits took 972ms probed
        // against 142ms driven.
        assert_eq!(plan.fetch_form(false, 3_843), Form::Driven);
        assert_eq!(plan.fetch_form(false, PROBED_FORM_LIMIT), Form::Driven);

        // Past the crossover the probe earns its place, and by an order of
        // magnitude: a word in most of the mailbox fills `LIMIT` after a
        // handful of rows, where driving walks every posting. `the` at the
        // 10,000 cap took 74ms probed against 1.26s driven.
        assert_eq!(plan.fetch_form(false, PROBED_FORM_LIMIT + 1), Form::Probed);
        assert_eq!(plan.fetch_form(false, TOTAL_HITS_CAP), Form::Probed);
    }

    #[test]
    fn ranking_by_relevance_always_drives_from_the_match() {
        // Whatever the count says. If a match is rankable at all it is narrow
        // enough that walking the postings is the cheap path, and the probed
        // shape cannot carry `bm25` scores in the first place.
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

    /// #746: hydrate's contacts probe must key on `address_normalized`, not
    /// only `account_id`. When the comparand was a nested correlated
    /// subquery, SQLite could not use it as an index key and fell back to
    /// walking every contact of the account once per hydrated candidate —
    /// `O(candidates × contacts)`, measured at ~14 ms per candidate against
    /// 18k contacts, which is 4.5 s for a 320-match query. The plan is the
    /// deterministic thing to pin: a timing assertion at that scale is a
    /// bench's job (`search_budget.rs` seeds contacts for exactly that).
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

        // Either the scoped index or the shared one, and either the
        // constraint's own index or its read companion: the claim is that the
        // probe is *keyed on the address*, not which of the four keys it.
        // The companions exist because this engine's planner will not read
        // through a partial index -- see
        // `turso_capabilities.rs::the_planner_does_not_use_a_partial_index`.
        assert!(
            steps
                .iter()
                .any(|step| step.contains("idx_contacts_") && step.contains("address_normalized=?")),
            "the contacts probe is not keyed on the address; plan:\n{steps:#?}"
        );
        assert!(
            !steps.iter().any(|step| step.starts_with("SCAN c")),
            "the contacts table is being scanned per candidate; plan:\n{steps:#?}"
        );
    }

    #[test]
    fn a_much_better_text_match_still_wins_over_recency_and_affinity() {
        let now = at(0);
        // A weak match, but very recent and from a frequent sender.
        let weak_but_boosted = rank_score(-0.1, at(0), now, 1000);
        // A strong text match, old and from a stranger.
        let strong_but_unboosted = rank_score(-20.0, at(365), now, 0);
        assert!(strong_but_unboosted < weak_but_boosted);
    }
}
