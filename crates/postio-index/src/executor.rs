//! Query execution: combining structured filters with the FTS5 index,
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
//! FTS5's `bm25()` scores relevance alone, and lower is better. That is not
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
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use postio_search::facets::{Facets, Refinement, Scope, ScopeCount};
use postio_search::query::{Filter, ParsedQuery, fts_literal};
use postio_search::results::{SearchHit, SearchResults, TOTAL_HITS_CAP};

use crate::error::Result;

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

/// [`CANDIDATE_POOL_MULTIPLIER`], for a recency-ordered fetch. See the
/// comment in [`search`] on `pool_size`.
const RECENCY_POOL_MULTIPLIER: u32 = 2;
/// [`CANDIDATE_POOL_MIN`], for a recency-ordered fetch.
const RECENCY_POOL_MIN: u32 = 50;

/// Recency's weight in [`rank_score`], relative to `bm25`'s native scale.
const RECENCY_WEIGHT: f64 = 2.0;
/// The age, in days, at which the recency boost has halved.
const RECENCY_HALF_LIFE_DAYS: f64 = 14.0;
/// Sender affinity's weight in [`rank_score`].
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
pub fn search(
    connection: &Connection,
    request: &SearchRequest<'_>,
    now: DateTime<Utc>,
) -> Result<SearchResults> {
    let start = Instant::now();
    let plan = Plan::build(request);

    let total_hits = plan.count(connection)?;
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
    let mut candidates = plan.fetch(connection, pool_size, rank_by_relevance)?;

    match request.order {
        postio_search::ResultOrder::Relevance => {
            for candidate in &mut candidates {
                candidate.score = rank_score(
                    candidate.bm25,
                    candidate.received_at,
                    now,
                    candidate.sender_times_seen,
                );
            }
            candidates.sort_by(|a, b| a.score.total_cmp(&b.score));
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
        hits,
        total_hits,
        total_hits_capped,
        elapsed,
    })
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
pub fn facets(connection: &Connection, request: &SearchRequest<'_>) -> Result<Facets> {
    // Scope counts hold the query and vary the scope: the column says what
    // *switching* would find, so it cannot be measured inside the scope the
    // user is already in.
    let mut scopes = Vec::with_capacity(Scope::ALL.len());
    for scope in Scope::ALL {
        let plan = Plan::build(&SearchRequest { scope, ..*request });
        scopes.push(ScopeCount {
            scope,
            hits: plan.count(connection)?,
        });
    }

    // Refinements are the opposite: they narrow what is on screen, so they
    // are measured inside the current scope.
    let plan = Plan::build(request);
    let mut refinements = plan.flag_refinements(connection)?;
    refinements.extend(plan.folder_refinements(connection)?);

    Ok(Facets {
        scopes,
        refinements,
    })
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
const HITS_JOIN: &str = "FROM (
             SELECT rowid AS rid, bm25(messages_fts) AS meta, NULL AS body
               FROM messages_fts WHERE messages_fts MATCH ?
             UNION ALL
             SELECT rowid, NULL, bm25(message_bodies_fts)
               FROM message_bodies_fts WHERE message_bodies_fts MATCH ?
          ) hits CROSS JOIN messages m ON m.id = hits.rid";

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
const CORRELATED_MATCH: &str =
    "(EXISTS (SELECT 1 FROM messages_fts WHERE rowid = m.id AND messages_fts MATCH ?)
   OR EXISTS (SELECT 1 FROM message_bodies_fts WHERE rowid = m.id AND message_bodies_fts MATCH ?))";

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
            snippet: self.snippet,
            score: self.score,
        }
    }
}

/// The account-scoped filter every search carries, plus the free-text `MATCH`
/// state, compiled once and shared by the count query and the fetch query.
struct Plan {
    conditions: Vec<String>,
    params: Vec<Value>,
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
    match_param: Option<Value>,
}

impl Plan {
    fn build(request: &SearchRequest<'_>) -> Self {
        // `AccountScope::Unified` names no account, so the predicate is
        // absent rather than widened -- which is why migration 0012 exists:
        // without `idx_messages_recency` the recency path has no index that
        // can supply its ordering once this conjunct is gone (ADR 0005 Q5a).
        let mut conditions = vec!["m.deleted_locally = 0".to_string()];
        let mut params: Vec<Value> = Vec::new();
        if let Some(id) = request.account.account() {
            conditions.push("m.account_id = ?".to_string());
            params.push(Value::Integer(id.get()));
        }
        let account = request.account;
        let mut has_match = false;
        let mut match_param = None;

        // Negated terms are excluded across the whole union rather than
        // folded into each index's own `MATCH`, and that is a correctness
        // fix rather than tidiness. `("report") NOT ("spam")` asked of
        // `messages_fts` is true for a message whose "spam" is in its *body*
        // — the metadata genuinely does not contain it — so the message came
        // back from a query that had explicitly refused it. An exclusion has
        // to be about the message, and only a condition outside the join can
        // be.
        for term in request.query.text_terms().filter(|term| term.negated) {
            conditions.push(
                "m.id NOT IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)
                 AND m.id NOT IN (SELECT rowid FROM message_bodies_fts
                                   WHERE message_bodies_fts MATCH ?)"
                    .to_string(),
            );
            let literal = Value::Text(fts_literal(&term.value));
            params.push(literal.clone());
            params.push(literal);
        }

        let positive = request
            .query
            .text_terms()
            .filter(|term| !term.negated)
            .map(|term| fts_literal(&term.value))
            .collect::<Vec<_>>();
        if !positive.is_empty() {
            let expr = positive.join(" AND ");
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
            match_param = Some(Value::Text(expr));
            has_match = true;
        }

        if let Some((sql, mut values)) = scope_condition(request.scope, request.account) {
            conditions.push(sql);
            params.append(&mut values);
        }

        for clause in request.query.filters() {
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
            match_param,
        }
    }

    /// How one statement reaches the free-text match.
    ///
    /// Not a tuning knob: the two forms have different *plans*, and which one
    /// is right depends on what the statement is about to do with the rows.
    /// See [`HITS_JOIN`] and [`CORRELATED_MATCH`].
    fn where_sql(&self, form: Form) -> String {
        let mut conditions = self.conditions.clone();
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

    /// Every parameter one statement binds, in the order its `?`s appear.
    ///
    /// The form decides that order, which is the whole reason this is not a
    /// field: a driven statement matches in its `FROM`, so its two
    /// expressions come first, and a probed one matches in its `WHERE`, so
    /// they come last.
    fn params_for(&self, form: Form) -> Vec<Value> {
        match form {
            Form::Driven => {
                let mut params = self.match_params();
                params.extend(self.params.iter().cloned());
                params
            }
            Form::Probed => {
                let mut params = self.params.clone();
                params.extend(self.match_params());
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
    fn match_params(&self) -> Vec<Value> {
        match &self.match_param {
            // Once for each index. The same expression: a term the user typed
            // is asked of the metadata and of the body, and either is a hit.
            Some(expr) => vec![expr.clone(), expr.clone()],
            None => Vec::new(),
        }
    }

    /// [`Plan::join_sql`], but for `fetch` specifically, where the join order
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
    fn fetch_form(&self, rank_by_relevance: bool) -> Form {
        if rank_by_relevance {
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
    fn count(&self, connection: &Connection) -> Result<u64> {
        let sql = format!(
            "SELECT count(*) FROM (SELECT DISTINCT m.id {} WHERE {} LIMIT ?)",
            self.source_sql(Form::Driven),
            self.where_sql(Form::Driven)
        );
        let mut params = self.params_for(Form::Driven);
        params.push(Value::Integer(TOTAL_HITS_CAP as i64));
        let count: i64 = connection.query_row(&sql, params_from_iter(&params), |row| row.get(0))?;
        Ok(count as u64)
    }

    /// The flag-shaped refinements — unread, flagged, attachments, size —
    /// counted over the match set in one pass.
    ///
    /// Conditional aggregates over a `LIMIT`ed subquery rather than four
    /// separate counts: the expensive part of any of these is walking the
    /// match, and walking it once for four answers is the whole point. The
    /// inner `LIMIT` is [`TOTAL_HITS_CAP`], the same bound [`Plan::count`]
    /// uses, so a very broad query cannot make the column expensive.
    ///
    /// The counts are therefore floors past the cap, exactly as
    /// [`SearchResults::total_hits`] is — and since [`Facets::suggested`]
    /// only ever compares them against that same capped total, a capped
    /// result set still ranks its refinements against each other correctly.
    fn flag_refinements(&self, connection: &Connection) -> Result<Vec<Refinement>> {
        let sql = format!(
            "SELECT
                 coalesce(sum(seen = 0), 0),
                 coalesce(sum(flagged), 0),
                 coalesce(sum(has_attachments), 0),
                 coalesce(sum(size >= ?), 0)
             FROM (SELECT DISTINCT m.id, m.seen, m.flagged, m.has_attachments, m.size
                     {from} WHERE {where_sql} LIMIT ?)",
            from = self.source_sql(Form::Driven),
            where_sql = self.where_sql(Form::Driven),
        );

        // The `size >= ?` bind sits before every condition's parameter,
        // because the aggregate is in the outer SELECT and the conditions are
        // in the subquery.
        let mut params = vec![Value::Integer(LARGE_BYTES as i64)];
        params.extend(self.params_for(Form::Driven));
        params.push(Value::Integer(TOTAL_HITS_CAP as i64));

        let counts: [i64; 4] = connection.query_row(&sql, params_from_iter(&params), |row| {
            Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?])
        })?;

        Ok(["is:unread", "is:flagged", "has:attach", LARGE_TOKEN]
            .into_iter()
            .zip(counts)
            .map(|(token, hits)| Refinement {
                token: token.to_string(),
                hits: hits.max(0) as u64,
            })
            .collect())
    }

    /// The folders the matches are actually in, as `in:` chips.
    ///
    /// Canvas 2b draws this chip as `list:lkml`; it is spelled `in:` here
    /// because `list:` cannot yet be answered exactly — see [`Scope::Lists`]
    /// and `postio-0bz`. `in:` names the same folder and is exact today, and
    /// the chip is a token the user could have typed either way.
    fn folder_refinements(&self, connection: &Connection) -> Result<Vec<Refinement>> {
        let sql = format!(
            "SELECT name, count(*) AS hits FROM (
                 SELECT DISTINCT m.id, mb.name AS name {from}
                   JOIN mailboxes mb ON mb.id = m.mailbox_id
                  WHERE {where_sql} LIMIT ?)
             GROUP BY name ORDER BY hits DESC, name LIMIT ?",
            from = self.source_sql(Form::Driven),
            where_sql = self.where_sql(Form::Driven),
        );

        let mut params = self.params_for(Form::Driven);
        params.push(Value::Integer(TOTAL_HITS_CAP as i64));
        params.push(Value::Integer(REFINE_FOLDERS as i64));

        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(&params), |row| {
            Ok(Refinement {
                token: format!("in:{}", quote_value(&row.get::<_, String>(0)?)),
                hits: row.get::<_, i64>(1)?.max(0) as u64,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Selects a candidate pool, then hydrates it into full [`Candidate`]s.
    ///
    /// Two queries rather than one: the first selects only `m.id`, ordered
    /// and cut down to `pool_size`, and is the query that has to be fast
    /// across every match size — the plan discussed in
    /// [`Plan::fetch_join_sql`] depends on the query being simple enough for
    /// SQLite to recognize. Folding in the per-row correlated subqueries
    /// (sender name/address, contact affinity, snippet) for *every* matching
    /// row, before the `LIMIT` narrows it, was measured to cost the same
    /// several hundred milliseconds on a broad match that `count` used to —
    /// even though the id-only shape alone was fast, adding those columns
    /// back to the same statement was enough to lose the plan again. Hydrating
    /// afterward, for only the (at most `pool_size`) ids that survive, keeps
    /// that cost paid once per candidate rather than once per match.
    fn fetch(
        &self,
        connection: &Connection,
        pool_size: u32,
        rank_by_relevance: bool,
    ) -> Result<Vec<Candidate>> {
        let scored = self.fetch_candidates(connection, pool_size, rank_by_relevance)?;
        self.hydrate(connection, &scored)
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
    fn fetch_candidates(
        &self,
        connection: &Connection,
        pool_size: u32,
        rank_by_relevance: bool,
    ) -> Result<Vec<(i64, f64)>> {
        let form = self.fetch_form(rank_by_relevance);
        // The row's own score on the driven path: without a `GROUP BY` each
        // arm of the union contributes its own row, so a message that matched
        // in both is ordered by its better half and summed below. That is a
        // *pool* ordering, not the answer — `search` re-ranks what comes back
        // through `rank_score` — so all it has to get right is which
        // candidates are worth hydrating.
        let order_by = if rank_by_relevance {
            "coalesce(hits.meta, 0.0) + coalesce(hits.body, 0.0)"
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
            (true, Form::Driven) => "hits.meta, hits.body",
            _ => "NULL, NULL",
        };
        let mut params: Vec<Value> = Vec::new();
        let sql = format!(
            "SELECT m.id, {scores} {from} WHERE {where_sql} ORDER BY {order_by} LIMIT ?",
            from = self.source_sql(form),
            where_sql = self.where_sql(form),
        );

        params.extend(self.params_for(form));
        // Asked for more than the pool, because the union can hand back the
        // same message twice and the duplicates are folded below. Doubling is
        // the bound: a message appears at most once per index.
        params.push(Value::Integer(i64::from(pool_size).saturating_mul(2)));

        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(&params), |row| {
            let meta: Option<f64> = row.get(1)?;
            let body: Option<f64> = row.get(2)?;
            Ok((
                row.get::<_, i64>(0)?,
                meta.unwrap_or(0.0) + BODY_SCORE_WEIGHT * body.unwrap_or(0.0),
            ))
        })?;

        // Folded here rather than with a `GROUP BY`, which would cost a sort
        // over the match set — the thing this whole shape exists to avoid.
        // Adding the two is what makes "matched in both indexes" the best a
        // message can do; `bm25` is negative-is-better, so each term pulls the
        // score down and a missing side adds nothing.
        let mut order: Vec<i64> = Vec::with_capacity(pool_size as usize);
        let mut scored: std::collections::HashMap<i64, f64> =
            std::collections::HashMap::with_capacity(pool_size as usize);
        for row in rows {
            let (id, score) = row?;
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
    fn hydrate(&self, connection: &Connection, scored: &[(i64, f64)]) -> Result<Vec<Candidate>> {
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
        let sql = format!(
            "SELECT
                 m.id, m.thread_id, m.mailbox_id, m.subject, m.received_at,
                 (SELECT name FROM recipients WHERE message_id = m.id AND kind = 'from'
                    ORDER BY position LIMIT 1) AS from_name,
                 (SELECT a.address FROM recipients r JOIN addresses a ON a.id = r.address_id
                    WHERE r.message_id = m.id AND r.kind = 'from'
                    ORDER BY r.position LIMIT 1) AS from_address,
                 (SELECT max(c.times_seen) FROM contacts c
                    WHERE c.address_normalized = (SELECT a.address_normalized
                                                    FROM recipients r
                                                    JOIN addresses a ON a.id = r.address_id
                                                   WHERE r.message_id = m.id AND r.kind = 'from'
                                                   ORDER BY r.position LIMIT 1)
                      {affinity}) AS sender_times_seen,
                 0 AS unused
             FROM messages m WHERE m.id IN ({placeholders})",
        );

        // Then one parameter per id in the `IN` list, after the affinity
        // subquery's own if it has one.
        let mut params = Vec::with_capacity(ids.len() + 1);
        if let Some(id) = self.account.account() {
            params.push(Value::Integer(id.get()));
        }
        params.extend(ids.iter().map(|id| Value::Integer(*id)));

        let mut statement = connection.prepare(&sql)?;
        let by_id: std::collections::HashMap<i64, Candidate> = statement
            .query_map(params_from_iter(&params), |row| {
                let id: i64 = row.get(0)?;
                Ok((
                    id,
                    Candidate {
                        message_id: MessageId::new(id),
                        thread_id: row.get::<_, Option<i64>>(1)?.map(ThreadId::new),
                        mailbox_id: MailboxId::new(row.get(2)?),
                        subject: row.get(3)?,
                        received_at: from_millis(row.get(4)?),
                        from_name: row.get(5)?,
                        from_address: row.get(6)?,
                        sender_times_seen: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                        // Filled in below, from the pool.
                        bm25: 0.0,
                        // Filled by whoever can read the body — see
                        // `SearchHit::snippet`.
                        snippet: String::new(),
                        score: 0.0,
                    },
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;

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
fn scope_condition(scope: Scope, account: AccountScope) -> Option<(String, Vec<Value>)> {
    let role = match scope {
        Scope::AllMail => return None,
        Scope::Inbox => "role = 'inbox'",
        Scope::Lists => "role = 'regular'",
    };
    // The role half is byte-for-byte the same in both scopes. Unified drops
    // the account conjunct and nothing else, which is what makes "every
    // account's inbox" a predicate removal rather than a redefinition of what
    // "Inbox" means (#186, ADR 0005 Q5a).
    Some(match account.account() {
        Some(id) => (
            format!("m.mailbox_id IN (SELECT id FROM mailboxes WHERE account_id = ? AND {role})"),
            vec![Value::Integer(id.get())],
        ),
        None => (
            format!("m.mailbox_id IN (SELECT id FROM mailboxes WHERE {role})"),
            Vec::new(),
        ),
    })
}

/// Translates one structured filter into a SQL condition (unnegated) plus its
/// bound parameters, in the order the `?` placeholders appear.
fn filter_condition(filter: &Filter) -> (String, Vec<Value>) {
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
            vec![Value::Text(value.clone()), Value::Text(value.clone())],
        ),
        Filter::In(value) => (
            "m.mailbox_id IN (SELECT id FROM mailboxes \
             WHERE lower(name) = lower(?) OR lower(path) = lower(?) OR role = lower(?))"
                .to_string(),
            vec![
                Value::Text(value.clone()),
                Value::Text(value.clone()),
                Value::Text(value.clone()),
            ],
        ),
        // ADR 0007 Q3: "from or to any member", resolved against `recipients`
        // by exact address rather than full text -- a group names people by
        // address, not by whatever words happen to appear near their name.
        // An unresolvable group name is an empty member set and therefore
        // matches nothing, the same "never everything" rule `Account` and
        // `In` follow just above.
        Filter::Group(value) => (
            "m.id IN (SELECT r.message_id FROM recipients r \
             JOIN addresses a ON a.id = r.address_id \
             WHERE r.kind IN ('from', 'to', 'cc', 'bcc') \
               AND a.address_normalized IN ( \
                 SELECT c.address_normalized FROM contact_group_members gm \
                 JOIN contacts c ON c.id = gm.contact_id \
                 JOIN contact_groups g ON g.id = gm.group_id \
                 WHERE lower(g.name) = lower(?)))"
                .to_string(),
            vec![Value::Text(value.clone())],
        ),
        Filter::Filename(value) => fts_column_condition("filenames", value),
        // `list:` names a mailing list by its `List-Id` (#9): the bracketed
        // identifier `postio-model::mime::list_id_from_text` extracts and
        // `messages.list_id` stores, indexed here the same way `subject` is.
        Filter::List(value) => fts_column_condition("list_id", value),
        Filter::HasAttachment => ("m.has_attachments = 1".to_string(), Vec::new()),
        Filter::Is(state) => {
            use postio_search::query::State;
            match state {
                State::Unread => ("m.seen = 0".to_string(), Vec::new()),
                State::Read => ("m.seen = 1".to_string(), Vec::new()),
                State::Flagged => ("m.flagged = 1".to_string(), Vec::new()),
            }
        }
        Filter::After(date) => (
            "m.received_at >= ?".to_string(),
            vec![Value::Integer(day_start_millis(*date))],
        ),
        Filter::Before(date) => (
            "m.received_at < ?".to_string(),
            vec![Value::Integer(day_start_millis(*date))],
        ),
        Filter::Larger(bytes) => (
            "m.size >= ?".to_string(),
            vec![Value::Integer(*bytes as i64)],
        ),
        Filter::Smaller(bytes) => (
            "m.size <= ?".to_string(),
            vec![Value::Integer(*bytes as i64)],
        ),
    }
}

/// Builds a condition against one `messages_fts` column, via a non-correlated
/// `IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ...)`
/// subquery — the same shape [`Plan::build`] uses to exclude negated-only
/// free text.
///
/// This is why `from:`/`to:`/`subject:`/`filename:`/`list:` match whole
/// tokens (as FTS5 tokenizes them) rather than an arbitrary substring: the
/// first version of this filter used `LIKE '%value%'` against `recipients`
/// and `attachments` directly, correlated to the outer message — cheap per
/// row, but `total_hits`'s `count(*)` has no `LIMIT` to short-circuit it, so
/// a plain `from:` search over a large mailbox paid for one such scan per
/// message in the account and blew the `<100 ms` budget (postio-y47's
/// benchmark caught this). Querying the column FTS5 already indexes turns
/// that into a single inverted-index lookup, the same cost class as free
/// text.
fn fts_column_condition(column: &str, value: &str) -> (String, Vec<Value>) {
    (
        "m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)".to_string(),
        vec![Value::Text(format!("{column}:{}", fts_literal(value)))],
    )
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

fn from_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(days_ago: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 23, 12, 0, 0).unwrap() - chrono::Duration::days(days_ago)
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
