//! What the relevance term's scale actually is, and what the boosts may do
//! against it (T036).
//!
//! `rank_score` blends three things: the text match, a recency boost and a
//! sender-affinity boost. The two boosts are weighted **relative to the
//! match's native scale** — and that scale is the engine's, not Postio's, so a
//! weight carried over from `bm25()` to `fts_score` is a number calibrated
//! against a quantity that no longer exists.
//!
//! This measures the scale and asserts what the blend must still do. It
//! prints, because the numbers in `RECENCY_WEIGHT`'s own documentation should
//! be checkable rather than trusted.
//!
//! # What was measured, and what it changed
//!
//! Run on the corpus below — 2,000 messages, term frequency 0..=5 crossed with
//! document length, which is what BM25 is a function of and what a uniform
//! corpus has none of:
//!
//! ```text
//! matches                             1,666
//! distinct scores                        15
//! whole range                 0.149 .. 0.351   spread 0.20
//! the best forty                      all tied
//! ```
//!
//! Two findings, and both change how the blend behaves.
//!
//! **The range is about a quarter of `bm25`'s.** `RECENCY_WEIGHT`'s own
//! documentation records a `bm25` spread of 0.80 across the top forty of a
//! real store. This is 0.20 across the *whole* match set of a fixture built to
//! maximise spread.
//!
//! **The best forty are tied.** The score is a function of term frequency and
//! length, and mail is full of near-identical subjects, so the top of a result
//! set is routinely one value. Whatever orders those rows, it is not the text
//! match.
//!
//! Together they mean the blend is **more recency-led than it was**, and there
//! is no weight that both orders old-against-new usefully and never overrides
//! a relevance gap: 0.20 is the entire relevance range, and a recency term
//! small enough to fit under it cannot separate last week from last year.
//!
//! The weights are unchanged, and that is the decision rather than the
//! default. #1216's complaint was search surfacing very old mail — recency
//! leading is what was asked for, and the case that still has to hold is the
//! narrow one: a *clearly* better match beats a *small* recency difference.
//! `executor::newest_order_answers_in_date_order_however_the_ranking_disagrees`
//! pins that with a 5×-density contrast five hours apart, and the second
//! assertion below pins it in the units this file measures.

use chrono::{TimeZone, Utc};
use postio_index::executor::rank_score;
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// Enough mail that a term's IDF is meaningful: over a two-document corpus
/// every score is the same for reasons that have nothing to do with ranking.
const MESSAGES: usize = 2_000;

/// How many of the best matches the top-of-set spread is measured over.
///
/// Forty because that is what the `bm25` calibration used, and a spread over
/// forty compared with a spread over four hundred is two different numbers.
const TOP: usize = 40;

fn at(hours: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 2, 0, 0, 0).unwrap() + chrono::Duration::hours(hours)
}

#[tokio::test]
async fn the_boosts_are_weighted_against_the_scale_the_engine_actually_produces() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    for n in 0..MESSAGES {
        let mut message = Message::new(account.id, inbox, at(-(n as i64)));
        message.from = vec![EmailAddress::new(
            Some("Ada Lovelace"),
            format!("sender{}@example.com", n % 50),
        )];
        // Term frequency and document length both vary, because BM25 is a
        // function of the two and a corpus that holds either still has no
        // spread to measure. Occurrences run 0..=5; the padding 0..=40 words.
        let occurrences = n % 6;
        let padding = (n % 9) * 5;
        let mut subject = String::new();
        for _ in 0..occurrences {
            subject.push_str("invoice ");
        }
        for word in 0..padding {
            subject.push_str(&format!("word{word} "));
        }
        subject.push_str(&format!("{n}"));
        message.subject = Some(subject);
        messages.create(&mut message).await.expect("create");
    }

    let scores: Vec<f64> = postio_storage::sql::all_unbounded(
        &connection,
        "SELECT fts_score(sender, recipients, subject, filenames, list_id, ?1) AS score
           FROM search_documents
          WHERE fts_match(sender, recipients, subject, filenames, list_id, ?1)
          ORDER BY score DESC",
        postio_storage::bind!["invoice"],
        |row| postio_storage::sql::RowExt::col::<f64>(row, 0),
    )
    .await
    .expect("scores");

    assert!(
        scores.len() > TOP * 4,
        "the corpus produced only {} matches; every number below would be \
         about the fixture rather than about the scorer",
        scores.len()
    );

    let best = scores.first().copied().unwrap_or(0.0);
    let worst = scores.last().copied().unwrap_or(0.0);
    let fortieth = scores.get(TOP - 1).copied().unwrap_or(0.0);
    let whole_range = best - worst;
    let top_range = best - fortieth;

    let mut distinct = scores.clone();
    distinct.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

    // The recency boost over a year, which is the separation a person would
    // expect to see honoured, and the most affinity can ever contribute.
    let recency_at = |days: f64| (-days / 730.0 * std::f64::consts::LN_2).exp();
    let recency_over_a_year = 3.0 * (recency_at(0.0) - recency_at(365.0));

    println!(
        "\n  {} matches, {} distinct scores\n  \
         whole range         {worst:.3} .. {best:.3}   spread {whole_range:.3}\n  \
         the best {TOP}         spread {top_range:.3}\n  \
         recency over a year        {recency_over_a_year:.3}\n  \
         affinity, at most          1.000\n",
        scores.len(),
        distinct.len(),
    );

    // 1. The scorer separates *something*. A single score for every match is
    //    not a narrow range, it is no ranking at all -- which is what a lost
    //    `fts_score` looks like, and what `turso_capabilities` pins two ways
    //    of causing.
    assert!(
        distinct.len() > 1 && whole_range > 0.0,
        "every one of {} matches scored the same. Relevance is not ordering \
         anything, and `rank_score` has been reduced to recency and affinity \
         -- see `turso_capabilities::the_score_is_lost_to_any_arithmetic_around_it`.",
        scores.len()
    );

    // 2. A clearly better match still wins against a small recency
    //    difference. This is the one the blend must not break, and it is
    //    stated in the units measured above rather than as a constant.
    let now = at(1);
    let best_but_a_week_old = rank_score(-best, at(-24 * 7), now, 0);
    let worst_but_today = rank_score(-worst, at(0), now, 0);
    assert!(
        best_but_a_week_old < worst_but_today,
        "the best match in the corpus, a week old, lost to the worst one from \
         today. The boosts are overriding relevance rather than nudging it, \
         and the lever is RECENCY_WEIGHT."
    );

    // 3. And recency can still reorder. A boost that cannot move anything is
    //    the fourteen-day half-life #1216 was about, where search answered
    //    with very old mail because the term was zero to four decimal places.
    let tied_and_old = rank_score(-best, at(-24 * 365 * 5), now, 0);
    let tied_and_new = rank_score(-best, at(0), now, 0);
    assert!(
        tied_and_new < tied_and_old,
        "two equally good matches five years apart ranked the same, so the \
         top of a result set -- where the scores are tied -- is in no \
         particular order at all"
    );

    // 4. Affinity does something at equal relevance and equal recency.
    let stranger = rank_score(-best, at(0), now, 0);
    let regular = rank_score(-best, at(0), now, 50);
    assert!(
        regular < stranger,
        "a correspondent seen fifty times ranked level with a stranger"
    );
}
