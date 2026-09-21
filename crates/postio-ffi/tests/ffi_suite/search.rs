//! Search, windowed like any other list.
//!
//! `PRODUCT.md` §1 puts finding things among the three jobs Postio must beat
//! the alternatives at, so a macOS build without search is missing a third of
//! the product rather than a feature.
//!
//! What these guard is that it is *the same* search. One query language, one
//! hit limit, one excerpt rule, one answer about what matched — the frontend
//! gets results, never the job of producing them.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// A store with three messages whose bodies are indexed, and its inbox.
async fn searchable() -> (std::sync::Arc<Session>, ScopeFfi) {
    let database = test_support::memory().await;
    let mailbox = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        // The FTS tables are the index's, created on demand rather than by a
        // store migration -- the body index stores no content (#407) and lives
        // beside the store rather than in it.
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index schema");
        let repository = MessageRepository::new(&connection);
        for (subject, body) in [
            ("Quarterly figures", "the quarterly numbers we discussed"),
            ("Lunch", "quarterly is not what this is about, lunch is"),
            ("Roadmap", "nothing in here says that word"),
        ] {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.subject = Some(subject.to_string());
            message.sync.body_state = BodyState::Full;
            repository.create(&mut message).await.expect("a message");
            repository
                .set_body(
                    message.id,
                    &StoredBody {
                        text: Some(body.to_string()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    BodyState::Full,
                )
                .await
                .expect("a body");
            postio_index::index::index_body(&connection, message.id.get(), Some(body))
                .await
                .expect("an indexed body");
        }
        inbox
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    let scope = ScopeFfi::Mailbox {
        mailbox: mailbox.into(),
    };
    session.open_scope(scope.clone());
    (session, scope)
}

/// The ids the window currently holds, once its pages have landed.
fn resident(session: &Session) -> Vec<i64> {
    let _ = session.row_at(0);
    session.settle_for_test();
    (0..session.row_count())
        .filter_map(|row| session.row_at(row))
        .map(|row| row.id)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_returns_hits_and_the_list_windows_over_them() {
    let (session, _) = searchable().await;
    session.search("quarterly").await;

    assert_eq!(session.row_count(), 2, "two messages say quarterly");
    assert_eq!(
        resident(&session).len(),
        2,
        "the rows never arrived, so the window is showing a count and nothing else"
    );
    assert!(session.is_searching());
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rows_come_back_in_rank_order_rather_than_by_date() {
    let (session, _) = searchable().await;
    session.search("quarterly").await;
    // The whole reason `message_rows` is used rather than a paged scope: a
    // ranked list re-sorted by date puts the best match wherever its date
    // happens to fall, which is the one thing a ranking must not do. The
    // subject match outranks the body-only one.
    let found = resident(&session);
    assert_eq!(found.len(), 2);
    let first = session.row_at(0).expect("the page landed");
    assert_eq!(
        first.subject.as_deref(),
        Some("Quarterly figures"),
        "the ranking did not survive the page read"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_matching_nothing_is_an_empty_list_rather_than_the_folder() {
    let (session, _) = searchable().await;
    session.search("zzzqqq").await;
    assert_eq!(session.row_count(), 0);
    assert!(
        session.is_searching(),
        "an empty result set is still a result set: falling back to the \
         folder would look like the query did nothing"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn operators_parse_the_way_the_shared_language_says() {
    let (session, _) = searchable().await;
    // Not a second parser. `postio-search` reads this on both platforms, and
    // the assertion that matters is that an *operator* reaches it rather than
    // being taken as a literal word -- `subject:quarterly` finding one row
    // and not two is what proves it was understood.
    session.search("subject:quarterly").await;
    assert_eq!(
        session.row_count(),
        1,
        "`subject:` was searched for as a word instead of read as an operator"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn clearing_restores_the_scope_that_was_open() {
    let (session, _) = searchable().await;
    let before = session.row_count();
    session.search("quarterly").await;
    assert_ne!(session.row_count(), before, "the search changed nothing");

    session.clear_search();
    assert!(!session.is_searching());
    assert_eq!(
        session.row_count(),
        before,
        "clearing did not come back to the folder that was open"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_query_still_comes_back_to_the_folder() {
    let (session, _) = searchable().await;
    let before = session.row_count();
    session.search("quarterly").await;
    // Typing again inside a search must not make the *first search* the thing
    // to return to, or `Escape` would walk backwards through every query.
    session.search("lunch").await;
    session.clear_search();
    assert_eq!(session.row_count(), before);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_folder_leaves_the_search_behind() {
    let (session, scope) = searchable().await;
    session.search("quarterly").await;
    session.open_scope(scope);
    assert!(!session.is_searching());
    // ...and clearing afterwards is a no-op rather than a jump back into a
    // search nobody is in.
    let showing = session.row_count();
    session.clear_search();
    assert_eq!(session.row_count(), showing);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn each_hit_carries_an_excerpt_with_the_match_located() {
    let (session, _) = searchable().await;
    session.search("quarterly").await;
    let first = resident(&session).first().copied().expect("a hit");

    let snippet = session.snippet_for(first).expect("a hit has an excerpt");
    assert!(!snippet.text.is_empty(), "an excerpt with no text in it");
    assert!(
        !snippet.ranges.is_empty(),
        "nothing was marked, so the frontend has nothing to highlight"
    );
    // Ranges, not markup: the text must be the plain string, and the marks
    // must point into it. A frontend given pre-marked text would be escaping
    // the other frontend's markup.
    assert!(
        !snippet.text.contains('<'),
        "the excerpt arrived marked up, which is a frontend's decision"
    );
    for range in &snippet.ranges {
        assert!(
            (range.end as usize) <= snippet.text.len(),
            "a match range points past the end of its own excerpt"
        );
        assert!(range.start < range.end, "an empty match range");
    }
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_row_that_is_not_a_hit_has_no_excerpt() {
    let (session, _) = searchable().await;
    assert_eq!(
        session.snippet_for(1),
        None,
        "an excerpt outside a search is a claim about a query nobody ran"
    );
    session.search("quarterly").await;
    assert_eq!(session.snippet_for(999_999), None);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn searching_drops_what_was_marked() {
    let (session, _) = searchable().await;
    session.invoke("next_message");
    let _ = session.row_at(0);
    session.settle_for_test();
    session.invoke("toggle_selection");
    assert!(!session.selected_messages().unwrap_or_default().is_empty());

    session.search("quarterly").await;
    assert_eq!(
        session.selected_messages(),
        Some(Vec::new()),
        "a selection survived into a list of different rows"
    );
    session.shutdown();
}

#[test]
fn the_query_reads_as_chips_from_one_parse() {
    // The chips are how somebody learns Postio's query language, so two
    // readings would be two languages on two platforms (#1157). The parse is
    // `postio-search`'s; both frontends draw the same one.
    let chips = postio_ffi::query_chips("from:ada has:attach quarterly".to_string());
    let labels: Vec<&str> = chips.iter().map(|chip| chip.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["from:ada", "has:attach"],
        "free text is not a chip: it is the part still being edited"
    );
    assert!(chips.iter().all(|chip| chip.complete));
    assert!(
        chips.iter().all(|chip| !chip.spoken.contains(':')),
        "a screen reader should hear `from Ada`, not `from colon ada`: {:?}",
        chips.iter().map(|c| &c.spoken).collect::<Vec<_>>()
    );
}

#[test]
fn a_half_typed_operator_is_still_a_chip() {
    // The moment the language is being learned: `from:` with no value tells
    // the user the parser understood the keyword.
    let chips = postio_ffi::query_chips("from:".to_string());
    assert_eq!(chips.len(), 1);
    assert!(!chips[0].complete);
}

#[test]
fn a_negated_operator_says_so() {
    let chips = postio_ffi::query_chips("-is:unread".to_string());
    assert_eq!(chips.len(), 1);
    assert!(chips[0].negated);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_search_reports_what_it_turned_out_to_be() {
    // Canvas 2b puts "14 hits · 11 ms" at the right-hand end of the field —
    // the 100ms budget made visible, which is a claim the application should
    // be willing to make on screen.
    let (session, _) = searchable().await;
    assert_eq!(
        session.search_outcome(),
        None,
        "there is no outcome before a search has run"
    );

    session.search("quarterly").await;
    let outcome = session.search_outcome().expect("a search just ran");
    assert_eq!(outcome.hits, 2, "two messages say quarterly");
    assert!(
        outcome.readout.contains("2 hits"),
        "the readout does not say how many: {}",
        outcome.readout
    );
    assert!(
        outcome.readout.contains("ms") || outcome.readout.contains("µs"),
        "the readout does not say how long: {}",
        outcome.readout
    );
    // The spoken form is words, because "·" and "ms" are punctuation and an
    // abbreviation rather than something to read out.
    assert!(!outcome.spoken.contains('·'), "spoken: {}", outcome.spoken);

    session.clear_search();
    assert_eq!(
        session.search_outcome(),
        None,
        "leaving search left a readout about a query nobody is running"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_that_matched_nothing_blames_the_query_and_not_the_mailbox() {
    // ADR 0005 Q10's worked example: somebody searches for an invoice, finds
    // nothing, and concludes it does not exist. The macOS list drew "This
    // store has no mail in it yet." over a zero-hit search -- over a mailbox
    // with three messages in it -- because the only empty-state branch it had
    // was keyed on the row count, and a search's row count is its hit count.
    let (session, scope) = searchable().await;
    session.open_scope(scope);

    assert!(
        session.empty_plate().is_none(),
        "a folder with rows in it needs no plate at all"
    );

    session.search("zzzznothingmatchesthis").await;
    assert_eq!(session.row_count(), 0, "the fixture matched something");

    let plate = session
        .empty_plate()
        .expect("a search that matched nothing has something to say");
    assert_eq!(plate.title, "No matches");
    assert!(
        plate.detail.contains("zzzznothingmatchesthis"),
        "the sentence has to name the query that found nothing: {}",
        plate.detail
    );
    assert!(
        !plate.detail.contains("no mail"),
        "the mailbox is not empty -- the query is: {}",
        plate.detail
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_the_search_leaves_its_sentence_behind_too() {
    // The plate is about the query, so it must not outlive it: `Escape`
    // restores the folder, and a folder with mail in it has nothing to say.
    let (session, scope) = searchable().await;
    session.open_scope(scope);
    session.search("zzzznothingmatchesthis").await;
    assert!(session.empty_plate().is_some());

    session.clear_search();
    assert!(
        session.empty_plate().is_none(),
        "the folder is back and it is not empty"
    );
    session.shutdown();
}

/// A corpus where relevance and date genuinely disagree, and the two
/// matching ids newest-first.
///
/// Built on the shape `index_suite`'s
/// `newest_order_answers_in_date_order_however_the_ranking_disagrees`
/// already proved, because two earlier attempts here did not disagree at
/// all and the test passed while proving nothing. Two things make the
/// difference, and both are properties of the ranker rather than of this
/// test: twenty non-matching messages, because BM25's IDF term goes to zero
/// when every document in the corpus matches; and **hours** between the two
/// matches rather than days, because `rank_score` folds recency in with a
/// calibrated weight and days of it outweigh any term density.
async fn disagreeing() -> (std::sync::Arc<Session>, Vec<i64>) {
    let database = test_support::memory().await;
    let (dense, recent) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index schema");
        let repository = MessageRepository::new(&connection);
        let base = Utc::now() - chrono::Duration::days(1);

        let write = async |subject: &str, body: &str, at: chrono::DateTime<Utc>| {
            let mut message = Message::new(account.id, inbox, at);
            message.subject = Some(subject.to_string());
            message.sync.body_state = BodyState::Full;
            repository.create(&mut message).await.expect("a message");
            repository
                .set_body(
                    message.id,
                    &StoredBody {
                        text: Some(body.to_string()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    BodyState::Full,
                )
                .await
                .expect("a body");
            postio_index::index::index_body(&connection, message.id.get(), Some(body))
                .await
                .expect("an indexed body");
            message.id.get()
        };

        for i in 0..20 {
            write(
                &format!("Entirely unrelated subject {i}"),
                "nothing in here says that word at all",
                base,
            )
            .await;
        }
        // Older, and saturated with the term: the far better match.
        let dense = write("Report", "report report report report report", base).await;
        // Newer by five hours, and a glancing match.
        let recent = write(
            "One report",
            "One report among other things entirely",
            base + chrono::Duration::hours(5),
        )
        .await;
        (dense, recent)
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    (session, vec![recent, dense])
}

#[tokio::test(flavor = "multi_thread")]
async fn results_can_be_read_newest_first_instead_of_best_first() {
    // `o` over a result set — #499's "the order of what I am looking at",
    // which is one idea and one key in both places it appears. Until now the
    // boundary answered every search in relevance order and had no way to be
    // asked for another, so `ToggleResultOrder` was a command macOS could
    // resolve and not obey.
    let (session, newest_first) = disagreeing().await;
    session.search("report").await;
    let by_relevance = resident(&session);

    session.toggle_result_order().await;

    assert_eq!(
        resident(&session),
        newest_first,
        "`o` did not put the results in date order, newest first"
    );
    assert_ne!(
        by_relevance, newest_first,
        "the two orders agree, so this fixture cannot tell them apart and \
         the assertion above proves nothing: {by_relevance:?}"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_order_survives_the_next_query() {
    // Having asked for date order, the *next* search is answered in date
    // order too. A toggle that reset itself on every query would be a
    // setting somebody has to re-press to keep, which is what makes it read
    // as broken rather than as a preference.
    let (session, _) = disagreeing().await;
    session.search("report").await;
    session.toggle_result_order().await;
    let by_date = resident(&session);

    session.search("report").await;
    assert_eq!(
        resident(&session),
        by_date,
        "the second search forgot the order the first one was left in"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn toggling_the_order_over_a_mailbox_does_nothing() {
    // Over a mailbox there is no other order to offer: the list is already
    // in the one order a mailbox has. GTK's control is inert there for the
    // same reason, and a key that quietly re-sorted somebody's inbox would
    // be a different command than the one they pressed.
    let (session, _) = searchable().await;
    let before = resident(&session);
    session.toggle_result_order().await;
    assert_eq!(
        resident(&session),
        before,
        "`o` re-sorted a mailbox, which has no result order to toggle"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_query_on_screen_can_be_read_back_to_be_saved() {
    // `⌘S` over results keeps *the query*, and the frontend does not hold
    // one: the field's text is whatever has been typed since, which may not
    // be what was run. What must be saved is the query that produced the
    // rows on screen, and the session is what knows it.
    let (session, _) = disagreeing().await;
    assert_eq!(
        session.search_query(),
        None,
        "a list showing a mailbox has no query to keep"
    );

    session.search("report").await;
    assert_eq!(session.search_query().as_deref(), Some("report"));

    session.clear_search();
    assert_eq!(
        session.search_query(),
        None,
        "the query outlived the search it belonged to"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sort_control_says_which_order_is_in_force() {
    // "Relevance ▾" on the canvas. The word is `ResultOrder::label`'s, so the
    // control and GTK's own say the same thing — and a control that did not
    // change when `o` did would be a label about the previous search.
    let (session, _) = disagreeing().await;
    session.search("report").await;
    assert_eq!(session.result_order_label(), "Relevance");

    session.toggle_result_order().await;
    assert_eq!(session.result_order_label(), "Newest");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_refine_chips_are_measured_against_the_results_on_screen() {
    // The discoverable half of the query language (#1157). Not a fixed list
    // typed into a frontend: a chip that keeps none of the current matches
    // is a dead end, and one that keeps all of them appears to do nothing
    // when clicked. `Facets::suggested` drops both, and the frontend draws
    // what survives.
    let (session, _) = disagreeing().await;
    session.search("report").await;

    let chips = session.refinements().await;
    assert!(
        chips.iter().all(|chip| chip.hits > 0),
        "a chip that keeps nothing was offered: {chips:?}"
    );
    assert!(
        chips.len() <= 4,
        "the shortlist is four; a column of twenty is a thing to read \
         rather than a thing to click: {}",
        chips.len()
    );
    assert!(
        chips.iter().all(|chip| !chip.token.is_empty()),
        "a chip with no token to append: {chips:?}"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mailbox_has_nothing_to_refine() {
    // The chips are about a result set. Over a mailbox there is none, and
    // offering `is:unread` there would be offering to search without saying
    // so.
    let (session, _) = searchable().await;
    assert!(session.refinements().await.is_empty());
    session.shutdown();
}
