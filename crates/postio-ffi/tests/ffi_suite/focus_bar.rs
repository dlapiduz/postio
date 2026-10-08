//! The command bar, go-to and the folders popover at the boundary
//! (specs/009-focus-macos T082, for the Mac's T084-T086).
//!
//! The controller decides (`postio-focus`'s `tests/bar.rs`); these assert the
//! Mac gets it through the session: `/` and `mod+k` open the bar as
//! `FocusOpenBar`, typed words come back as `FocusBarLines`, a line runs by
//! its token, and the folders popover lists the places one read found and
//! opens the one chosen, which the list then shows.

use postio_ffi::{BarLineKindFfi, BarModeFfi, BarSelectFfi, Session, SessionOptions, UiEvent};

use crate::focus::{cursor_on_the_first_row, heard, inbox_of};

/// Wait for the bar's next lines that `wanted` accepts, and hand them back.
async fn lines(
    session: &Session,
    mut wanted: impl FnMut(&postio_ffi::BarViewFfi) -> bool,
) -> postio_ffi::BarViewFfi {
    let mut seen = None;
    assert!(
        heard(session, 10, |event| match event {
            UiEvent::FocusBarLines { view } if wanted(view) => {
                seen = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "the bar never drew the lines wanted"
    );
    seen.expect("seen")
}

#[tokio::test(flavor = "multi_thread")]
async fn slash_opens_the_bar_and_in_lists_a_folders_conversations() {
    let session = inbox_of(&["First", "Second"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("search");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenBar {
                mode: BarModeFfi::Search,
                text,
                select: None,
            } if text.is_empty()
        ))
        .await,
        "`/` opens the bar for search"
    );
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusPlacesChanged
        ))
        .await,
        "the places are read as it opens"
    );
    session.focus_bar_typed("in:Inbox".to_owned());
    let view = lines(&session, |view| view.heading.is_some()).await;
    assert_eq!(
        view.heading.as_deref(),
        Some("Inbox \u{b7} folder \u{b7} 2 conversations \u{b7} newest first")
    );
    let subjects: Vec<&str> = view
        .lines
        .iter()
        .filter(|line| line.kind == BarLineKindFfi::Message)
        .map(|line| line.title.as_str())
        .collect();
    assert_eq!(subjects, ["First", "Second"], "newest first");
    assert!(
        view.lines
            .iter()
            .filter(|line| line.kind == BarLineKindFfi::Message)
            .all(|line| line.selectable && line.sender.as_deref() == Some("Ada")),
        "{:?}",
        view.lines
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_run_from_the_bar_acts_on_the_cursors_row() {
    let session = inbox_of(&["First", "Second"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("command_palette");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenBar {
                mode: BarModeFfi::Commands,
                text,
                ..
            } if text == ">"
        ))
        .await,
        "mod+k opens the bar on commands, `>` typed (C24)"
    );
    session.focus_bar_typed(">mark read".to_owned());
    let view = lines(&session, |view| {
        view.lines
            .iter()
            .any(|line| line.command.as_deref() == Some("toggle_read"))
    })
    .await;
    let line = view
        .lines
        .iter()
        .find(|line| line.command.as_deref() == Some("toggle_read"))
        .expect("the row");
    assert_eq!(line.kind, BarLineKindFfi::Command);
    assert!(line.key.is_some(), "a command's row shows its key");
    session.focus_bar_run(line.token);
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusCloseSurface {
                kind: postio_ffi::SurfaceKindFfi::Bar
            }
        ))
        .await,
        "running a line closes the bar"
    );
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusToast { undoable: true, .. }
        ))
        .await,
        "and the verb ran, as the host says"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_folders_popover_lists_the_places_and_opens_one() {
    let session = inbox_of(&["First"]).await;
    cursor_on_the_first_row(&session).await;

    session.invoke("go_to_folders");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenPlaces
        ))
        .await,
        "`g o` opens the popover"
    );
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusPlacesChanged
        ))
        .await,
        "and the places are read"
    );
    let places = session.focus_places(String::new());
    let names: Vec<&str> = places.iter().map(|place| place.name.as_str()).collect();
    // Filtering is on by default, so Filtered is a place too.
    assert_eq!(
        names,
        ["Inbox", "Snoozed", "Filtered", "Flagged"],
        "{places:?}"
    );
    let inbox = &places[0];
    assert_eq!(inbox.count.as_deref(), Some("1"), "conversations, counted");
    assert_eq!(inbox.section, "Mailboxes");
    assert_eq!(inbox.command.as_deref(), Some("go_to_inbox"), "its key's");
    assert!(inbox.footer.contains("in:Inbox"), "{}", inbox.footer);
    assert_eq!(
        session.focus_places("snoo".to_owned()).len(),
        1,
        "filtered as typed"
    );

    let snoozed = places
        .iter()
        .find(|place| place.name == "Snoozed")
        .expect("Snoozed");
    session.focus_open_place(snoozed.token);
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusPlace { name } if name == "Snoozed"
        ))
        .await,
        "the strip names the place"
    );
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusListChanged { total: 0 }
        ))
        .await,
        "and the list shows it"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_search_runs_on_its_key_and_mod_s_saves_the_query() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "[saved_searches.budget]\nquery = \"subject:budget\"\npinned = true\n",
    )
    .expect("a config");
    let session = Session::open(SessionOptions::in_memory().with_config_file_for_test(&path))
        .expect("a session");

    session.invoke("saved_search_1");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenBar { text, .. } if text == "subject:budget"
        ))
        .await,
        "alt+1 opens the bar on the first saved search"
    );
    let view = lines(&session, |view| !view.saved.is_empty()).await;
    assert_eq!(view.saved, ["budget"], "the saved row names it");

    session.focus_bar_typed("from:ada invoice".to_owned());
    let view = lines(&session, |view| view.chips.len() == 2).await;
    assert_eq!(view.chips, ["from:ada", "invoice"]);
    assert!(session.focus_bar_tab(), "Tab steps into the chips");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusOpenBar {
                select: Some(BarSelectFfi { start: 0, end: 8 }),
                ..
            }
        ))
        .await,
        "the first chip is selected in the field"
    );

    session.invoke("save_search");
    assert!(
        heard(&session, 5, |event| matches!(
            event,
            UiEvent::FocusToast { text, .. }
                if *text == postio_ui::focus_target::search_saved("from:ada invoice")
        ))
        .await,
        "mod+s saves the bar's query and says so"
    );
    let written = std::fs::read_to_string(&path).expect("the config");
    assert!(written.contains("from:ada invoice"), "{written}");
    assert!(
        written.contains("subject:budget"),
        "nothing else lost: {written}"
    );
    session.shutdown();
}

#[test]
fn tab_with_no_bar_is_not_used() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    assert!(!session.focus_bar_tab(), "the toolkit's Tab, then");
    session.shutdown();
}
