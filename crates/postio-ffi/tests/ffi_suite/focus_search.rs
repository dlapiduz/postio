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
