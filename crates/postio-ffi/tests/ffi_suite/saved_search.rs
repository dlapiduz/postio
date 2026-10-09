//! Keeping a query, across the boundary.
//!
//! The surface #1574 is about: the macOS search field draws *Save search as
//! folder*, enabled, and pressing it did nothing. These assertions are all of
//! the form "the file on disk says so afterwards", because a verb that
//! returns a plausible list without writing anything is exactly the failure
//! that shipped — a frontend cannot tell the difference, and neither can a
//! test that only reads the return value.

use postio_ffi::{save_search, saved_searches};

/// A file with the things a careless rewrite destroys, and one search in it.
const SAMPLE: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[saved_searches.urgent]
query = \"is:unread\"
pinned = true
";

/// A `config.toml` in a directory of its own, holding `text`.
fn config(text: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).expect("the sample config");
    (dir, path.display().to_string())
}

#[test]
fn a_saved_query_becomes_a_row_the_sidebar_can_draw() {
    let (dir, path) = config("");

    let edit = save_search(path.clone(), "is:unread from:team".to_owned()).expect("a first save");

    assert_eq!(
        edit.changed.as_deref(),
        Some("is-unread-from-team"),
        "the caller needs the key of the row it just made"
    );
    assert_eq!(edit.searches.len(), 1);
    assert_eq!(edit.searches[0].query, "is:unread from:team");
    assert_eq!(
        edit.searches[0].name, "is-unread-from-team",
        "a search nobody has renamed draws under its key"
    );

    // The half that matters: it is in the file, so the next launch has it.
    assert_eq!(
        saved_searches(path),
        edit.searches,
        "the verb answered with a list it had not written"
    );
    drop(dir);
}

#[test]
fn a_machine_with_no_config_file_yet_has_no_saved_searches() {
    // First run, before the settings panel has ever been opened. Not an
    // error, and not a reason for the sidebar to show anything at all.
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("nothing-here.toml").display().to_string();

    assert!(saved_searches(path.clone()).is_empty());

    // And saving still works: the file is created by the first save.
    let edit = save_search(path.clone(), "has:attach".to_owned()).expect("a save creates the file");
    assert_eq!(edit.changed.as_deref(), Some("has-attach"));
    assert_eq!(saved_searches(path).len(), 1);
}

#[test]
fn saving_an_empty_query_is_not_a_row_and_not_an_error() {
    // Pinning an empty query pins "everything", which is not a folder anybody
    // meant to make. The affordance stays pressable; it just declines.
    let (dir, path) = config(SAMPLE);

    let edit = save_search(path.clone(), "   ".to_owned()).expect("an empty query is not a fault");

    assert_eq!(edit.changed, None);
    assert_eq!(
        edit.searches.len(),
        1,
        "the existing row still has to be drawable: {:?}",
        edit.searches
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml")).expect("the file"),
        SAMPLE,
        "a verb that did nothing rewrote the file"
    );
}

#[test]
fn the_rest_of_the_file_survives_a_saved_search() {
    // ADR 0031's load-bearing clause. `config.toml` is a file a person edits
    // by hand, and saving a search is not permission to reformat it.
    let (dir, path) = config(SAMPLE);

    save_search(path, "has:attach".to_owned()).expect("the sample parses");

    let after = std::fs::read_to_string(dir.path().join("config.toml")).expect("the file");
    assert!(
        after.contains("# hand-written, and it should stay that way"),
        "the comment did not survive:\n{after}"
    );
    assert!(after.contains("idle = true"), "[sync] moved:\n{after}");
    assert!(
        after.contains("[saved_searches.has-attach]"),
        "and the save itself still has to land:\n{after}"
    );
}

#[test]
fn a_config_that_will_not_parse_is_refused_and_left_alone() {
    // The tempting fallback -- a broken file tells us nothing, so start from
    // the defaults -- would write an empty `[saved_searches]` over searches the user
    // still has. Refusing is the only reading that cannot lose them.
    let broken =
        "[ui]\ndensity = 42\n\n[saved_searches.keep]\nquery = \"is:unread\"\npinned = true\n";
    let (dir, path) = config(broken);

    let refused = save_search(path.clone(), "has:attach".to_owned());
    assert!(refused.is_err(), "a file that does not parse was rewritten");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml")).expect("the file"),
        broken,
        "the file was touched anyway"
    );
}

/// ⌘S in Focus's results, then Save ↩ with Notify and Keep the date
/// rolling on (spec 010 US5, FR-029): the session's own `config.toml`
/// holds the search afterwards, named, notifying, its date relative.
#[tokio::test(flavor = "multi_thread")]
async fn the_save_popover_writes_name_notify_and_a_rolling_date_to_config_toml() {
    use postio_ffi::{DropdownStateFfi, Session, SessionOptions, UiEvent};

    use crate::focus::heard;

    let (dir, path) = config("[sync]\nidle = true\n");
    let (database, _) = postio_demo::seeded(postio_demo::Seed::Search).await;
    let session = Session::open(
        SessionOptions::in_memory_with(database)
            .with_config_file_for_test(std::path::Path::new(&path)),
    )
    .expect("a session");
    session.invoke("search");
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusDropdown { view } if view.state == DropdownStateFfi::Empty
        ))
        .await
    );
    session.focus_bar_typed("atlas budget after:2026-07-01".to_owned());
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusDropdown { view } if view.footer_count.is_some()
        ))
        .await
    );
    session.focus_search_show_all();
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusResults { view } if !view.groups.is_empty()
        ))
        .await
    );

    session.invoke("save_search");
    let mut shown = None;
    assert!(
        heard(&session, 10, |event| match event {
            UiEvent::FocusSavePopover { view: Some(view) } => {
                shown = Some(view.clone());
                true
            }
            _ => false,
        })
        .await,
        "⌘S hangs the Save popover"
    );
    let view = shown.expect("seen");
    assert_eq!(view.name, "Atlas budget");
    assert_eq!(view.chips, ["after:2026-07-01", "atlas budget"]);
    assert!(view.pin && !view.notify && !view.rolling);
    assert!(
        view.rolling_note
            .as_deref()
            .is_some_and(|note| note.starts_with("Off: always since 1 July.")),
        "{:?}",
        view.rolling_note
    );
    assert!(
        std::fs::read_to_string(&path)
            .expect("the file")
            .lines()
            .all(|line| !line.contains("saved_searches")),
        "nothing is written before Save"
    );

    session.focus_search_save("Atlas, rolling".to_owned(), true, true, true);
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusSavePopover { view: None }
        ))
        .await,
        "Save takes the popover down"
    );
    let written = std::fs::read_to_string(&path).expect("the file");
    let read = postio_config::Config::from_toml_str(&written).expect("still parses");
    let (_, saved) = read
        .filters
        .iter()
        .find(|(_, filter)| filter.name.as_deref() == Some("Atlas, rolling"))
        .unwrap_or_else(|| panic!("the search is in the file:\n{written}"));
    assert!(saved.notify, "notify = true:\n{written}");
    assert!(saved.pinned);
    assert!(
        saved.query.starts_with("atlas budget after:") && saved.query.ends_with('d'),
        "its date is relative: {}",
        saved.query
    );
    assert!(
        written.starts_with("[sync]\nidle = true\n"),
        "the rest as it was"
    );
    drop(dir);
}
