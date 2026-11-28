//! POSTIO-MEASUREMENT: reports the engine's own scale, read off a built index (57 s), so it runs on
//! the nightly timer.

use chrono::{TimeZone, Utc};
use postio_index::executor::rank_score;
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

const MESSAGES: usize = 2_000;

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

    assert!(
        distinct.len() > 1 && whole_range > 0.0,
        "every one of {} matches scored the same. Relevance is not ordering \
         anything, and `rank_score` has been reduced to recency and affinity \
         -- see `turso_capabilities::the_score_is_lost_to_any_arithmetic_around_it`.",
        scores.len()
    );

    let now = at(1);
    let best_but_a_week_old = rank_score(-best, at(-24 * 7), now, 0);
    let worst_but_today = rank_score(-worst, at(0), now, 0);
    assert!(
        best_but_a_week_old < worst_but_today,
        "the best match in the corpus, a week old, lost to the worst one from \
         today. The boosts are overriding relevance rather than nudging it, \
         and the lever is RECENCY_WEIGHT."
    );

    let tied_and_old = rank_score(-best, at(-24 * 365 * 5), now, 0);
    let tied_and_new = rank_score(-best, at(0), now, 0);
    assert!(
        tied_and_new < tied_and_old,
        "two equally good matches five years apart ranked the same, so the \
         top of a result set -- where the scores are tied -- is in no \
         particular order at all"
    );

    let stranger = rank_score(-best, at(0), now, 0);
    let regular = rank_score(-best, at(0), now, 50);
    assert!(
        regular < stranger,
        "a correspondent seen fifty times ranked level with a stranger"
    );
}
