//! The search dropdown at the boundary (specs/010-focus-search step 2).
//!
//! The controller decides (`postio-focus`'s `tests/bar.rs`, the dropdown
//! section); these assert the Mac gets it through the session: `/` emits
//! `FocusDropdown` with the recent and saved searches read from the store
//! and the config, ⌥⌫ forgets a recent in the store, and typing quickly
//! completes only the searches still wanted, each drawn in order.

use std::time::Duration;

use postio_ffi::{
    DropdownRowKindFfi, DropdownStateFfi, DropdownViewFfi, Session, SessionOptions, UiEvent,
};
use postio_storage::searches::SearchRepository;

use crate::focus::heard;

/// Wait for the next dropdown that `wanted` accepts.
async fn dropdown(
    session: &Session,
    secs: u64,
    mut wanted: impl FnMut(&DropdownViewFfi) -> bool,
) -> DropdownViewFfi {
    let mut seen = None;
    assert!(
        heard(session, secs, |event| match event {
            UiEvent::FocusDropdown { view } if wanted(view) => {
                seen = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "the dropdown never drew what was wanted"
    );
    seen.expect("seen")
}

fn runs(runs: &[postio_ffi::RunFfi]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

/// A store with two searches run, and a config with one saved search.
async fn with_history() -> std::sync::Arc<Session> {
    let database = postio_storage::test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        // An index, empty: what the searches run against.
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the search index");
        let searches = SearchRepository::new(&connection);
        let now = chrono::Utc::now();
        searches
            .remember("from:ada invoice", 6, now - chrono::TimeDelta::days(3))
            .await
            .expect("remembered");
        searches
            .remember("atlas budget", 48, now - chrono::TimeDelta::hours(2))
            .await
            .expect("remembered");
    }
    Session::open(
        SessionOptions::in_memory_with(database).with_config_for_test(
            "[saved_searches.atlas]\nquery = \"subject:atlas\"\npinned = true\nname = \"Atlas\"\n",
        ),
    )
    .expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_empty_bar_emits_a_dropdown_with_recents_and_saved_searches() {
    let session = with_history().await;
    session.invoke("search");
    let view = dropdown(&session, 10, |view| {
        view.state == DropdownStateFfi::Empty
            && view.sections.iter().any(|section| {
                !section.rows.is_empty() && section.rows[0].kind == DropdownRowKindFfi::Recent
            })
            && view
                .sections
                .iter()
                .any(|section| section.pills.iter().any(|pill| pill.count.is_some()))
    })
    .await;
    let titles: Vec<&str> = view
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect();
    assert_eq!(titles, ["Recent", "Saved searches", "Search by"]);
    let recent: Vec<String> = view.sections[0]
        .rows
        .iter()
        .map(|row| runs(&row.title))
        .collect();
    assert_eq!(recent, ["atlas budget", "from:ada invoice"], "newest first");
    assert_eq!(runs(&view.sections[0].rows[0].detail), "48 results");
    assert_eq!(view.highlight, Some(view.sections[0].rows[0].token));
    let saved = &view.sections[1].pills[0];
    assert_eq!(saved.label, "Atlas");
    assert_eq!(
        saved.count.as_deref(),
        Some("0"),
        "nothing in this store matches it"
    );
    assert!(saved.key.is_some(), "its ⌥ number");
    assert_eq!(
        view.sections[2].rows.len(),
        9,
        "eight operators and the example"
    );
    assert!(!view.footer_hints.is_empty());

    // ⌥⌫ on the newest: gone from the panel, and from the store.
    session.focus_search_forget(view.sections[0].rows[0].token);
    let view = dropdown(&session, 5, |view| {
        view.sections
            .first()
            .is_some_and(|section| section.rows.len() == 1)
    })
    .await;
    assert_eq!(runs(&view.sections[0].rows[0].title), "from:ada invoice");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn show_all_keeps_the_query_among_the_recent_searches() {
    let session = with_history().await;
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    session.focus_bar_typed("harbor".to_owned());
    let view = dropdown(&session, 10, |view| view.footer_count.is_some()).await;
    assert_eq!(view.state, DropdownStateFfi::Words);
    session.focus_search_show_all();
    // Closed and opened again: the query is the newest recent search.
    session.invoke("back");
    tokio::time::sleep(Duration::from_millis(100)).await;
    session.invoke("search");
    let view = dropdown(&session, 10, |view| {
        view.sections
            .first()
            .and_then(|section| section.rows.first())
            .is_some_and(|row| runs(&row.title) == "harbor")
    })
    .await;
    assert_eq!(runs(&view.sections[0].rows[0].detail), "0 results");
    session.shutdown();
}

/// Typing `atlas budget` a letter at a time, faster than a search answers
/// (D8): the driver aborts a lane's superseded task, so at most two
/// conversation searches run to the end -- the last, and one that won the
/// race -- not one a keystroke, and the panel is drawn for the words typed
/// last only. (Spec 010's `a`, `at`, `atl`, `atla` alone: "at" is plain
/// English's, and two words that find nothing are too quick to race.)
#[tokio::test(flavor = "multi_thread")]
async fn typing_quickly_completes_only_the_searches_still_wanted() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    let before = session.focus_search_reads_for_test();

    let typed = "atlas budget";
    for end in 1..=typed.len() {
        session.focus_bar_typed(typed[..end].to_owned());
    }
    let mut counts = Vec::new();
    let _ = heard(&session, 3, |event| {
        if let UiEvent::FocusDropdown { view } = event
            && let Some(count) = &view.footer_count
        {
            counts.push(count.clone());
        }
        false
    })
    .await;
    assert!(!counts.is_empty(), "the last words were answered");
    assert!(
        counts[0] != "0 matches \u{b7} <1 ms",
        "and found: {counts:?}"
    );
    assert!(
        counts.windows(2).all(|pair| pair[0] == pair[1]),
        "one answer drawn, then only its passages: {counts:?}"
    );
    let completed = session.focus_search_reads_for_test() - before;
    assert!(
        (1..=2).contains(&completed),
        "{completed} conversation searches ran to the end for twelve keystrokes"
    );
    session.shutdown();
}

/// Wait for the next event that `pick` takes.
async fn next<T>(session: &Session, secs: u64, mut pick: impl FnMut(&UiEvent) -> Option<T>) -> T {
    let mut seen = None;
    assert!(
        heard(session, secs, |event| {
            seen = pick(event);
            seen.is_some()
        })
        .await,
        "the event never came"
    );
    seen.expect("seen")
}

/// ⌘↩ over the search seed turns the main window into the results (spec
/// 010 step 3): `FocusQuery` says what the field holds, `FocusResults`
/// the frame with its groups, and the rows read back with the query's
/// words marked; a filter button edits the query into a chip; Esc leaves.
#[tokio::test(flavor = "multi_thread")]
async fn show_all_emits_the_results_and_their_rows_read_back_marked() {
    use postio_ffi::{FilterKindFfi, ResultsViewFfi, TermEditFfi};

    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    session.focus_bar_typed("atlas budget".to_owned());
    let _ = dropdown(&session, 10, |view| view.footer_count.is_some()).await;
    session.focus_search_show_all();

    let query = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(query.words, "atlas budget");
    assert!(query.chips.is_empty());
    assert_eq!(query.buttons.len(), 8);

    let view: ResultsViewFfi = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if !view.groups.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert!(view.groups[0].top_hits, "Best match: Top hits first");
    assert!(view.groups[0].rows <= 3);
    assert!(view.groups.len() > 1, "then the months");
    assert_eq!(session.focus_search_row_count(), view.rows);
    assert!(
        view.count_line.ends_with("conversations"),
        "{}",
        view.count_line
    );
    assert!(view.footer_right.contains("local index"));
    assert_eq!(view.months.len(), 12);
    assert!(
        view.months
            .iter()
            .all(|bar| bar.count == bar.conversations.to_string()),
        "a month's count crosses as said, exact below the walk's cap (D30): {:?}",
        view.months
    );

    // The rows' passages land after their pages; read every row then.
    let _ = heard(&session, 3, |event| {
        matches!(event, UiEvent::FocusResultsPage { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut marked = Vec::new();
    for position in 0..view.rows.min(20) {
        let Some(row) = session.focus_search_row(position) else {
            continue;
        };
        assert!(!row.accessible.is_empty());
        for run in row.subject.iter().chain(&row.passage) {
            if run.highlighted {
                marked.push(run.text.to_lowercase());
            }
        }
    }
    assert!(marked.iter().any(|word| word == "atlas"), "{marked:?}");
    assert!(marked.iter().any(|word| word == "budget"), "{marked:?}");

    // A filter button: the query grows a chip, and the button turns solid.
    session.focus_search_edit(TermEditFfi::Toggle {
        field: "has".to_owned(),
        value: "attachment".to_owned(),
    });
    let query = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } if !view.chips.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        (
            query.chips[0].operator.as_str(),
            query.chips[0].value.as_str()
        ),
        ("has:", "attachment")
    );
    assert!(
        query
            .buttons
            .iter()
            .any(|button| button.kind == FilterKindFfi::Attachment && button.applied)
    );

    // Esc with nothing selected: the inbox.
    session.invoke("back");
    next(&session, 10, |event| {
        matches!(event, UiEvent::FocusLeaveResults).then_some(())
    })
    .await;
    session.shutdown();
}

/// The results view's chrome crosses as `postio-ui` words, so the Mac has
/// no literal of its own for any of them (§3.1-3.5).
#[test]
fn the_results_chrome_words_cross_from_postio_ui() {
    let words = postio_ffi::focus_search_words();
    assert_eq!(words.back, "Inbox");
    assert_eq!(words.save, "Save search");
    assert_eq!(words.sort, "Sort");
    assert_eq!(words.best_match, "Best match");
    assert_eq!(words.newest, "Newest");
    assert_eq!(
        words.timeline_hint,
        "Matches by month \u{b7} drag across months to narrow"
    );
    assert_eq!(postio_ffi::focus_search_checked(5), "5 selected");
    // Design §5: what a screen reader calls each surface, and a dropdown
    // row's sentence.
    assert_eq!(words.query_label, "Search query");
    assert_eq!(words.suggestions_label, "Search suggestions");
    assert_eq!(words.results_label, "Search results");
    assert_eq!(words.files_label, "Files");
    assert_eq!(words.people_label, "People");
    assert_eq!(
        postio_ffi::focus_search_row_accessible(
            "Ada Moreno".into(),
            "ada@example.com".into(),
            None,
            Some("yesterday".into())
        ),
        "Ada Moreno, ada@example.com, yesterday"
    );
}

/// The search seed's results for "atlas budget", Best match: the session
/// and the frame, once it has groups.
async fn atlas_budget_results() -> (std::sync::Arc<Session>, postio_ffi::ResultsViewFfi) {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    session.focus_bar_typed("atlas budget".to_owned());
    let _ = dropdown(&session, 10, |view| view.footer_count.is_some()).await;
    session.focus_search_show_all();
    let view = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if !view.groups.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    (session, view)
}

/// Every top hit matched in its subject and its body, and shows the body's
/// passage with the words marked (design §3.4: "every row has a
/// passage"), not the subject again and not nothing. The seed's top hits
/// are one-line messages whose match is in their first line.
#[tokio::test(flavor = "multi_thread")]
async fn a_top_hit_matched_in_its_body_shows_the_bodys_passage_marked() {
    let (session, view) = atlas_budget_results().await;
    assert!(view.groups[0].top_hits);
    let top = view.groups[0].rows;
    assert!(top > 0);

    // Passages land after their page: wait for them, a little.
    let deadline = std::time::Instant::now() + postio_test_support::scaled(Duration::from_secs(5));
    let rows = loop {
        let rows: Vec<_> = (0..top)
            .filter_map(|position| session.focus_search_row(position))
            .collect();
        let landed = rows.len() as u64 == top && rows.iter().all(|row| !row.passage.is_empty());
        if landed || std::time::Instant::now() > deadline {
            break rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    for row in &rows {
        assert!(
            row.passage.iter().any(|run| run.highlighted),
            "{} ({}): passage {:?}",
            runs(&row.subject),
            row.source_tag,
            runs(&row.passage)
        );
        assert!(
            !runs(&row.passage).contains("wrote:"),
            "{}",
            runs(&row.passage)
        );
        // The tag is the passage's source; the subject is the first line's.
        assert!(
            !row.source_tag.contains("subject"),
            "{}: tagged {:?}",
            runs(&row.subject),
            row.source_tag
        );
    }
    session.shutdown();
}

/// A result's label pill crosses with the label's own colour, as the
/// list's pills do, so its dot is the label's and not grey (design §3.4:
/// the orange dot for Atlas).
#[tokio::test(flavor = "multi_thread")]
async fn a_results_label_pill_carries_the_labels_colour() {
    let (session, view) = atlas_budget_results().await;
    let pill = (0..view.rows.min(20))
        .filter_map(|position| session.focus_search_row(position))
        .flat_map(|row| row.pills)
        .find(|pill| pill.name == "Atlas")
        .expect("an Atlas row among the first");
    // The seed gives Atlas the design's colour.
    assert_eq!(pill.color.as_deref(), Some("#c08a2e"));
    session.shutdown();
}

/// A filter popover at the boundary (spec 010 step 4, FR-027): From opens
/// with the people the results hold, a check previews, and Esc puts the
/// query back: the last `FocusQuery` is the one it opened on.
#[tokio::test(flavor = "multi_thread")]
async fn a_popover_closed_with_esc_puts_back_the_query_it_opened_on() {
    use postio_ffi::FilterKindFfi;

    let (session, _) = atlas_budget_results().await;
    session.focus_search_popover(FilterKindFfi::From);
    // From is ringed while its popover is open.
    let opened = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view }
            if view
                .buttons
                .iter()
                .any(|button| button.kind == FilterKindFfi::From && button.open) =>
        {
            Some(view.clone())
        }
        _ => None,
    })
    .await;
    let popover = next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: Some(view) } if !view.rows.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(popover.kind, FilterKindFfi::From);
    assert_eq!(popover.placeholder, "Filter people in these results");
    assert!(popover.rows.iter().all(|row| !row.checked && row.count > 0));
    assert!(
        popover
            .rows
            .iter()
            .all(|row| row.count_label == row.count.to_string()),
        "a row's count crosses as drawn, exact below the walk's cap (D30)"
    );

    session.focus_search_popover_toggle(popover.rows[0].token, false);
    let checked = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } if !view.chips.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(checked.chips[0].operator, "from:");

    session.focus_search_popover_done(false);
    next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: None } => Some(()),
        _ => None,
    })
    .await;
    let restored = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } if view.chips.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    let mut expected = opened.clone();
    for button in &mut expected.buttons {
        button.open = false;
    }
    assert_eq!(restored, expected);
    session.shutdown();
}

/// D27 end to end: with a person applied, reopening From lists everyone the
/// rest of the query finds, the applied person checked -- the real engine
/// answers the facets-only read.
#[tokio::test(flavor = "multi_thread")]
async fn a_popover_reopened_with_its_field_applied_still_lists_everyone() {
    use postio_ffi::FilterKindFfi;

    let (session, _) = atlas_budget_results().await;
    session.focus_search_popover(FilterKindFfi::From);
    let first = next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: Some(view) } if !view.rows.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert!(first.rows.len() >= 2, "a second person to check");

    session.focus_search_popover_toggle(first.rows[0].token, false);
    next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } if !view.chips.is_empty() => Some(()),
        _ => None,
    })
    .await;
    session.focus_search_popover_done(true);
    next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: None } => Some(()),
        _ => None,
    })
    .await;

    session.focus_search_popover(FilterKindFfi::From);
    let again = next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: Some(view) } if !view.rows.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        again.rows.len(),
        first.rows.len(),
        "everyone the query without from: finds"
    );
    assert_eq!(again.rows.iter().filter(|row| row.checked).count(), 1);
    session.shutdown();
}

/// The timeline's months and the Date popover's words at the boundary
/// (FR-023, FR-027): bars 9 to 11 become `after:` and `before:` chips,
/// and "since july" in the Date popover says what it became.
#[tokio::test(flavor = "multi_thread")]
async fn months_and_date_words_cross_as_dates() {
    use postio_ffi::FilterKindFfi;

    let (session, view) = atlas_budget_results().await;
    assert_eq!(
        view.timeline_hint,
        "Matches by month \u{b7} drag across months to narrow"
    );
    session.focus_search_months(9, 11);
    let query = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } if view.chips.len() == 2 => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        query
            .chips
            .iter()
            .map(|chip| chip.operator.as_str())
            .collect::<Vec<_>>(),
        ["after:", "before:"]
    );
    let narrowed = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if view.months.iter().any(|bar| bar.selected) => {
            Some(view.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(narrowed.months.iter().filter(|bar| bar.selected).count(), 3);
    assert!(narrowed.timeline_step.is_some());

    session.focus_search_popover(FilterKindFfi::Date);
    let date = next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: Some(view) } => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(date.presets.len(), 6);
    assert_eq!(date.presets[5].count, None, "Custom… has no count");
    session.focus_search_date_words("since july".to_owned());
    let date = next(&session, 10, |event| match event {
        UiEvent::FocusPopover { view: Some(view) } if view.parsed.is_some() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert!(
        date.parsed
            .as_deref()
            .is_some_and(|parsed| parsed.starts_with("\u{2192} after:")),
        "{:?}",
        date.parsed
    );
    session.focus_search_popover_done(true);
    session.shutdown();
}

/// Quick Look at the boundary (spec 010 step 5, FR-028): Space on a result
/// says `FocusQuickLook` with the panel, its cards land as a second one
/// with the conversation's matches, ] rings the next, and Space again
/// closes it with `None`.
#[tokio::test(flavor = "multi_thread")]
async fn quick_look_crosses_with_its_cards_and_closes_with_none() {
    let (session, _) = atlas_budget_results().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    session.invoke("quick_look");
    let view = next(&session, 10, |event| match event {
        UiEvent::FocusQuickLook { view: Some(view) } if view.cards.len() > 1 => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(view.title, "Quick Look");
    assert!(view.position.starts_with("1 of "), "{}", view.position);
    assert!(runs(&view.subject).to_lowercase().contains("atlas"));
    assert!(view.subject.iter().any(|run| run.highlighted));
    assert!(
        view.matches_line.ends_with("matches in this conversation"),
        "{}",
        view.matches_line
    );
    assert_eq!(
        view.actions
            .iter()
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>(),
        ["Open", "Archive", "Close"]
    );
    let current = view.current.expect("a card is ringed");
    assert!(
        view.cards
            .iter()
            .any(|card| card.passage.iter().any(|run| run.highlighted)),
        "{:?}",
        view.cards
    );

    session.invoke("next_match");
    let moved = next(&session, 10, |event| match event {
        UiEvent::FocusQuickLook { view: Some(view) } => Some(view.current),
        _ => None,
    })
    .await;
    assert_eq!(moved, Some(current + 1));

    session.invoke("quick_look");
    let _ = next(&session, 10, |event| match event {
        UiEvent::FocusQuickLook { view: None } => Some(()),
        _ => None,
    })
    .await;
    session.shutdown();
}

/// Spec 010 step 7 (US6, FR-030, screen 13): a search the seed holds
/// nothing for crosses as `FocusRelaxations` -- the page at once, still
/// counting, then its ways out numbered most first with their queries --
/// the field's hint names ⌘⌫'s key, and 1 runs the first way out, which
/// takes the page away with `None`.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_found_crosses_as_the_no_results_page_and_a_number_runs_a_way_out() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    session.focus_bar_typed(
        "from:ada@example.com has:attachment before:2026-08-01 subject:budget".to_owned(),
    );
    let _ = dropdown(&session, 10, |view| view.footer_count.is_some()).await;
    session.focus_search_show_all();

    let page = next(&session, 10, |event| match event {
        UiEvent::FocusRelaxations { view: Some(view) } => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(page.title, "Nothing matches all four filters");
    // Attachment contents are claimed only once the indexer has been
    // through every file on this machine (step 9), which here races the
    // search: either sentence is the truth at the moment it is said.
    assert!(
        page.searched.starts_with("Searched all ")
            && (page.searched.ends_with(" on this Mac.")
                || page
                    .searched
                    .ends_with(" on this Mac, including attachment contents.")),
        "{}",
        page.searched
    );
    let page = if page.counting.is_some() {
        next(&session, 30, |event| match event {
            UiEvent::FocusRelaxations { view: Some(view) } if view.counting.is_none() => {
                Some(view.clone())
            }
            _ => None,
        })
        .await
    } else {
        page
    };
    assert!(!page.relaxations.is_empty(), "{page:?}");
    let numbers: Vec<u32> = page.relaxations.iter().map(|way| way.number).collect();
    assert_eq!(numbers, (1..=numbers.len() as u32).collect::<Vec<_>>());
    assert!(page.relaxations[0].focused);
    assert_eq!(page.relaxations[0].key.as_deref(), Some("1"));
    let first = page.relaxations[0].query.clone();

    session.invoke("pick_relaxation_1");
    // The page goes, the field holds the looser query with its usual
    // hint, and the results land -- in whatever order they cross.
    let (mut gone, mut hint, mut rows) = (false, None, 0);
    assert!(
        heard(&session, 30, |event| {
            match event {
                UiEvent::FocusRelaxations { view: None } => gone = true,
                UiEvent::FocusQuery { view } => {
                    hint = Some((view.hint.clone(), view.hint_key.clone()))
                }
                UiEvent::FocusResults { view } => rows = view.rows,
                _ => {}
            }
            gone && rows > 0
        })
        .await,
        "the way out never ran"
    );
    assert_eq!(hint, Some(("/ to edit".to_owned(), None)));
    assert!(rows > 0, "{first} finds something");
    session.shutdown();
}

/// Spec 010 step 9 (US8, FR-031, FR-053): over the search seed with its
/// files on this machine, ⌘2 shows the Files tab -- its header and cards
/// read back with `focus_search_file` -- and Space on a workbook hands
/// `FocusFileCopy` a copy in the app's own temporary folder, which
/// `focus_search_file_done` removes.
#[tokio::test(flavor = "multi_thread")]
async fn the_files_tab_crosses_as_cards_and_space_hands_over_a_copy() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let scratch = tempfile::tempdir().expect("scratch");
    let blobs = postio_storage::BlobStore::open(
        scratch.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    postio_demo::store_search_blobs(&blobs).expect("the seed's files");
    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    session.focus_bar_typed("atlas budget".to_owned());
    let _ = dropdown(&session, 10, |view| view.footer_count.is_some()).await;
    session.focus_search_show_all();
    let _ = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if !view.groups.is_empty() => Some(()),
        _ => None,
    })
    .await;

    session.invoke("results_files");
    let view = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if view.files.is_some() && view.rows > 0 => {
            Some(view.clone())
        }
        _ => None,
    })
    .await;
    let header = view.files.expect("the Files tab's header");
    assert_eq!(header.title, "Files whose name or contents match");
    assert!(view.groups.is_empty());
    let cards: Vec<postio_ffi::FileCardFfi> = (0..view.rows)
        .map(|at| session.focus_search_file(at).expect("a card"))
        .collect();
    assert!(session.focus_search_file(view.rows).is_none());
    assert!(cards[0].focused);
    let workbook = cards
        .iter()
        .position(|card| card.kind == "XLSX")
        .expect("a workbook among the cards");
    assert!(runs(&cards[workbook].name).to_lowercase().contains("atlas"));
    assert!(cards[workbook].subject.starts_with("in \u{2018}"));

    session.focus_search_point(workbook as u64);
    session.invoke("quick_look");
    let copy = next(&session, 10, |event| match event {
        UiEvent::FocusFileCopy { copy: Some(copy) } => Some(copy.clone()),
        _ => None,
    })
    .await;
    assert!(!copy.save);
    let path = std::path::PathBuf::from(&copy.path);
    assert!(path.starts_with(postio_focus::file_copies()), "{path:?}");
    assert!(path.exists(), "the copy is there for Quick Look");
    assert_eq!(
        copy.name,
        cards[workbook]
            .name
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>()
    );

    session.focus_search_file_done();
    let gone = async {
        for _ in 0..100 {
            if !path.exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    };
    assert!(gone.await, "the copy is removed when the panel is gone");
    session.shutdown();
}

/// Spec 010 step 10 (US9, FR-032, design §3.11): over the search seed,
/// ⌘3 shows the People tab -- its rows read back with
/// `focus_search_person`, you never among them -- and ↩ on a person runs
/// `from:` them on the Conversations tab.
#[tokio::test(flavor = "multi_thread")]
async fn the_people_tab_crosses_as_rows_and_return_searches_their_mail() {
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    session.invoke("search");
    let _ = dropdown(&session, 10, |view| view.state == DropdownStateFfi::Empty).await;
    session.focus_bar_typed("atlas".to_owned());
    let _ = dropdown(&session, 10, |view| view.footer_count.is_some()).await;
    session.focus_search_show_all();
    let _ = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if !view.groups.is_empty() => Some(()),
        _ => None,
    })
    .await;

    session.invoke("results_people");
    let view = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if view.tabs[2].selected && view.rows > 0 => {
            Some(view.clone())
        }
        _ => None,
    })
    .await;
    assert!(view.groups.is_empty());
    assert_eq!(view.tabs[2].count, view.rows.to_string());
    let people: Vec<postio_ffi::PersonRowFfi> = (0..view.rows)
        .map(|at| session.focus_search_person(at).expect("a person"))
        .collect();
    assert!(session.focus_search_person(view.rows).is_none());
    assert!(people[0].focused);
    assert!(
        people
            .iter()
            .all(|person| person.address != "you@example.com"),
        "never yourself: {people:?}"
    );
    let ada = people
        .iter()
        .find(|person| person.address == "ada@example.com")
        .expect("Ada wrote about Atlas");
    assert_eq!(ada.name, "Ada Moreno");
    assert_eq!(ada.initials, "AM");
    assert!(ada.messages.ends_with("messages"), "{}", ada.messages);
    assert!(!ada.last.is_empty());

    let at = people
        .iter()
        .position(|person| person.address == "ada@example.com")
        .unwrap();
    session.focus_search_point(at as u64);
    session.invoke("open_message");
    let query = next(&session, 10, |event| match event {
        UiEvent::FocusQuery { view } if !view.chips.is_empty() => Some(view.clone()),
        _ => None,
    })
    .await;
    assert_eq!(query.chips.len(), 1);
    assert_eq!(query.chips[0].operator, "from:");
    let _ = next(&session, 10, |event| match event {
        UiEvent::FocusResults { view } if view.tabs[0].selected && view.rows > 0 => Some(()),
        _ => None,
    })
    .await;
    session.shutdown();
}
