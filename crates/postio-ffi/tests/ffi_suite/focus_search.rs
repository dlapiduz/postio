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
