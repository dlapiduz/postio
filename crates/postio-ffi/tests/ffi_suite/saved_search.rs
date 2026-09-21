//! Keeping a query, across the boundary.
//!
//! The surface #1574 is about: the macOS search field draws *Save search as
//! folder*, enabled, and pressing it did nothing. These assertions are all of
//! the form "the file on disk says so afterwards", because a verb that
//! returns a plausible list without writing anything is exactly the failure
//! that shipped — a frontend cannot tell the difference, and neither can a
//! test that only reads the return value.

use postio_ffi::{
    ReorderFfi, delete_saved_search, move_saved_search, rename_saved_search, save_search,
    saved_search_delete_prompt, saved_search_rename_prompt, saved_searches,
};

/// A file with the things a careless rewrite destroys, and one search in it.
const SAMPLE: &str = "\
# hand-written, and it should stay that way
[sync]
idle = true

[filters.urgent]
query = \"is:unread\"
pinned = true
";

/// Three pinned rows, so that the front, the middle and the back are three
/// different places. One row is its own first and last, which makes every
/// reorder over it a refusal and tells nothing about the direction that was
/// asked for.
const THREE: &str = "\
[filters.a]
query = \"from:ada\"
pinned = true

[filters.b]
query = \"from:grace\"
pinned = true

[filters.c]
query = \"from:alan\"
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
fn renaming_changes_the_label_and_never_the_key() {
    // #292: the key is the identity every other verb names. A rename that
    // moved it would orphan the row the moment anything held onto one.
    let (dir, path) = config(SAMPLE);

    let edit = rename_saved_search(
        path.clone(),
        "urgent".to_owned(),
        "Needs a reply".to_owned(),
    )
    .expect("the sample parses");

    assert_eq!(edit.changed.as_deref(), Some("urgent"));
    assert_eq!(edit.searches[0].name, "Needs a reply");
    assert_eq!(edit.searches[0].key, "urgent");
    assert_eq!(saved_searches(path)[0].name, "Needs a reply");
    drop(dir);
}

#[test]
fn renaming_a_search_that_is_not_there_changes_nothing() {
    let (dir, path) = config(SAMPLE);

    let edit = rename_saved_search(path, "no-such-search".to_owned(), "Whatever".to_owned())
        .expect("a missing key is not a fault");

    assert_eq!(edit.changed, None);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml")).expect("the file"),
        SAMPLE
    );
}

#[test]
fn moving_a_search_reorders_the_rows_and_says_which_one_moved() {
    let (dir, path) = config(THREE);

    let edit = move_saved_search(path.clone(), "c".to_owned(), ReorderFfi::Up)
        .expect("three rows, and c is not first");

    assert_eq!(
        edit.changed.as_deref(),
        Some("c"),
        "the keyboard has to follow the row, not the position it left"
    );
    let keys: Vec<&str> = edit.searches.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["a", "c", "b"]);

    // Persisted, or the row springs back on the next launch.
    let keys: Vec<String> = saved_searches(path.clone())
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["a", "c", "b"]);

    // And the far end is not a fault, it is simply nothing.
    let edit = move_saved_search(path, "a".to_owned(), ReorderFfi::Up).expect("a is already first");
    assert_eq!(edit.changed, None);
    drop(dir);
}

#[test]
fn moving_a_search_down_walks_the_other_way() {
    // `MoveSavedSearchUp` and `MoveSavedSearchDown` are two commands that
    // differ by one enum value, and this boundary is where that value is
    // translated from the frontend's spelling into the shared one. A `Down`
    // that arrives as an `Up` still moves the row, still persists, and still
    // answers with the key it moved: nothing downstream can tell, and neither
    // can an assertion on `changed`. The order the rows come back in is the
    // only thing that can.
    let (dir, path) = config(THREE);

    let edit = move_saved_search(path.clone(), "a".to_owned(), ReorderFfi::Down)
        .expect("three rows, and a is not last");

    assert_eq!(edit.changed.as_deref(), Some("a"));
    let keys: Vec<&str> = edit.searches.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["b", "a", "c"], "down is not up");

    let keys: Vec<String> = saved_searches(path.clone())
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["b", "a", "c"], "and it has to survive the write");

    // The refusal belongs to the end this direction is walking toward, not to
    // the row: `c` is last, so it declines `Down` while `a` a moment ago did
    // not.
    let edit =
        move_saved_search(path, "c".to_owned(), ReorderFfi::Down).expect("c is already last");
    assert_eq!(edit.changed, None);
    drop(dir);
}

#[test]
fn deleting_takes_the_row_away() {
    let (dir, path) = config(SAMPLE);

    let edit =
        delete_saved_search(path.clone(), "urgent".to_owned()).expect("the sample has that key");

    assert_eq!(edit.changed.as_deref(), Some("urgent"));
    assert!(edit.searches.is_empty());
    assert!(saved_searches(path).is_empty());
    assert!(
        !std::fs::read_to_string(dir.path().join("config.toml"))
            .expect("the file")
            .contains("[filters.urgent]")
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
        after.contains("[filters.has-attach]"),
        "and the save itself still has to land:\n{after}"
    );
}

#[test]
fn a_config_that_will_not_parse_is_refused_and_left_alone() {
    // The tempting fallback -- a broken file tells us nothing, so start from
    // the defaults -- would write an empty `[filters]` over searches the user
    // still has. Refusing is the only reading that cannot lose them.
    let broken = "[ui]\ndensity = 42\n\n[filters.keep]\nquery = \"is:unread\"\npinned = true\n";
    let (dir, path) = config(broken);

    let refused = save_search(path.clone(), "has:attach".to_owned());
    assert!(refused.is_err(), "a file that does not parse was rewritten");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml")).expect("the file"),
        broken,
        "the file was touched anyway"
    );
    assert!(delete_saved_search(path, "keep".to_owned()).is_err());
}

#[test]
fn both_platforms_ask_the_same_questions() {
    // The words, not the dialog. A destructive verb whose confirmation was
    // written twice is two confirmations, and the one nobody is looking at is
    // the one that says the wrong thing about what is lost.
    let delete = saved_search_delete_prompt();
    assert_eq!(delete.title, "Delete this saved search?");
    assert_eq!(delete.confirm, "Delete");
    assert_eq!(delete.cancel, "Keep");
    assert!(
        delete
            .body
            .as_deref()
            .is_some_and(|body| body.contains("query")),
        "the body has to say what is actually lost: {delete:?}"
    );

    let rename = saved_search_rename_prompt();
    assert_eq!(rename.title, "Rename this saved search?");
    assert_eq!(rename.confirm, "Rename");
    assert_eq!(
        rename.body, None,
        "the pre-filled entry says everything a sentence would"
    );
}
