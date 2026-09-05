//! The composer's editing undo — the stack that is *not* `postio_core::undo`.

use postio_body::{Document, EditHistory};

fn doc(text: &str) -> Document {
    Document::from_text(text)
}

#[test]
fn undo_and_redo_walk_the_history() {
    let mut history = EditHistory::new();
    history.record(doc("a"), doc("ab"));
    history.record(doc("ab"), doc("abc"));

    assert_eq!(history.undo(), Some(doc("ab")));
    assert_eq!(history.undo(), Some(doc("a")));
    assert_eq!(history.undo(), None, "walked past the beginning");

    assert_eq!(history.redo(), Some(doc("ab")));
    assert_eq!(history.redo(), Some(doc("abc")));
    assert_eq!(history.redo(), None);
}

#[test]
fn a_typing_run_amends_rather_than_stacking() {
    // Undo per keystroke is not undo, it is a way to lose an afternoon one
    // character at a time.
    let mut history = EditHistory::new();
    history.record(doc(""), doc("h"));
    for run in ["he", "hel", "hell", "hello"] {
        assert!(history.amend(doc(run)));
    }

    assert_eq!(
        history.undo(),
        Some(doc("")),
        "the whole run is one step, and it undoes to where the run started"
    );
    assert!(!history.can_undo());
    assert_eq!(history.redo(), Some(doc("hello")), "and redoes to its end");
}

#[test]
fn amending_nothing_says_so_rather_than_inventing_a_step() {
    let mut history = EditHistory::new();
    assert!(!history.amend(doc("x")));
    assert!(!history.can_undo());
}

#[test]
fn a_change_that_changes_nothing_is_not_a_step() {
    // Every autosave tick and every focus change re-reads the buffer. Without
    // this, `Ctrl+Z` appears to do nothing several times before it does
    // something, which reads as a broken key.
    let mut history = EditHistory::new();
    history.record(doc("same"), doc("same"));

    assert!(!history.can_undo());
}

#[test]
fn typing_after_an_undo_discards_the_branch() {
    // What every editor does: once you type after undoing, the thing you
    // undid is gone rather than lurking behind a redo.
    let mut history = EditHistory::new();
    history.record(doc("a"), doc("ab"));
    history.undo();
    assert!(history.can_redo());

    history.record(doc("a"), doc("ax"));

    assert!(!history.can_redo(), "the undone branch survived a new edit");
    assert_eq!(history.undo(), Some(doc("a")));
}

#[test]
fn the_history_is_bounded() {
    let mut history = EditHistory::new();
    for n in 0..EditHistory::DEPTH + 50 {
        history.record(doc(&n.to_string()), doc(&(n + 1).to_string()));
    }

    let mut walked = 0;
    while history.undo().is_some() {
        walked += 1;
    }
    assert_eq!(
        walked,
        EditHistory::DEPTH,
        "a draft edited all afternoon must not grow without limit"
    );
}

#[test]
fn clearing_forgets_both_directions() {
    let mut history = EditHistory::new();
    history.record(doc("a"), doc("ab"));
    history.undo();

    history.clear();

    assert!(!history.can_undo() && !history.can_redo());
}
