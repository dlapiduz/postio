use postio_body::{
    DocPosition, Document, EditHistory, EditIntent, EditStep, Join, Selection, apply,
};

fn doc(text: &str) -> Document {
    Document::from_text(text)
}

fn at(offset: u32) -> Selection {
    Selection::caret(DocPosition::new([0], offset))
}

/// A whole-document change, as a surface that only sees snapshots records it.
fn step(before: &str, after: &str) -> EditStep {
    EditStep::between(&doc(before), &doc(after), at(0), at(after.len() as u32))
}

/// Undoes into `current` and says what it became.
fn undo(history: &mut EditHistory, current: &mut Document) -> Option<Document> {
    history.undo(current).map(|_| current.clone())
}

fn redo(history: &mut EditHistory, current: &mut Document) -> Option<Document> {
    history.redo(current).map(|_| current.clone())
}

#[test]
fn undo_and_redo_walk_the_history() {
    let mut history = EditHistory::new();
    history.record(step("a", "ab"), Join::New);
    history.record(step("ab", "abc"), Join::New);
    let mut current = doc("abc");

    assert_eq!(undo(&mut history, &mut current), Some(doc("ab")));
    assert_eq!(undo(&mut history, &mut current), Some(doc("a")));
    assert_eq!(
        undo(&mut history, &mut current),
        None,
        "walked past the beginning"
    );

    assert_eq!(redo(&mut history, &mut current), Some(doc("ab")));
    assert_eq!(redo(&mut history, &mut current), Some(doc("abc")));
    assert_eq!(redo(&mut history, &mut current), None);
}

#[test]
fn a_typing_run_extends_one_step_until_the_surface_seals_it() {
    let mut history = EditHistory::new();
    for (before, after) in [("", "h"), ("h", "he"), ("he", "hel")] {
        history.record(step(before, after), Join::Run);
    }
    history.seal();
    history.record(step("hel", "help"), Join::Run);
    let mut current = doc("help");

    assert_eq!(
        undo(&mut history, &mut current),
        Some(doc("hel")),
        "the sealed run ended"
    );
    assert_eq!(
        undo(&mut history, &mut current),
        Some(doc("")),
        "the first run is one step, back to where it started"
    );
    assert!(!history.can_undo());
    assert_eq!(
        redo(&mut history, &mut current),
        Some(doc("hel")),
        "and redoes to its end"
    );
}

#[test]
fn a_new_step_is_never_extended_by_the_run_after_it() {
    let mut history = EditHistory::new();
    history.record(step("a", "ab"), Join::Run);
    history.record(step("ab", "abc"), Join::New);
    history.record(step("abc", "abcd"), Join::Run);
    let mut current = doc("abcd");

    assert_eq!(undo(&mut history, &mut current), Some(doc("abc")));
    assert_eq!(
        undo(&mut history, &mut current),
        Some(doc("ab")),
        "the conversion is its own"
    );
}

#[test]
fn undo_gives_back_the_caret_from_before_the_step() {
    let start = doc("ab");
    let edited = apply(&start, &at(1), EditIntent::SplitBlock);
    let mut history = EditHistory::new();
    history.record(
        EditStep::between(&start, &edited.document, at(1), edited.selection.clone()),
        edited.join,
    );
    let mut current = edited.document.clone();

    let undone = history.undo(&mut current).unwrap();
    assert_eq!(current, start);
    assert_eq!(undone.selection, at(1));
    let redone = history.redo(&mut current).unwrap();
    assert_eq!(current, edited.document);
    assert_eq!(redone.selection, edited.selection);
}

#[test]
fn an_in_place_edit_records_only_the_blocks_it_replaced() {
    let mut current = Document::from_text("one\n\ntwo\n\nthree");
    let before = at(0);
    let mid = Selection::caret(DocPosition::new([1], 3));
    let applied = current.edit(&mid, EditIntent::Insert("!".into()));
    let step = EditStep::of(&applied, &current, before);
    assert_eq!((step.at, step.before.len(), step.after.len()), (1, 1, 1));

    let mut history = EditHistory::new();
    history.record(step, applied.join);
    history.undo(&mut current).unwrap();
    assert_eq!(current, Document::from_text("one\n\ntwo\n\nthree"));
}

#[test]
fn a_change_that_changes_nothing_is_not_a_step() {
    let mut history = EditHistory::new();
    history.record(step("same", "same"), Join::New);

    assert!(!history.can_undo());
}

#[test]
fn typing_after_an_undo_discards_the_branch() {
    let mut history = EditHistory::new();
    history.record(step("a", "ab"), Join::New);
    let mut current = doc("ab");
    history.undo(&mut current);
    assert!(history.can_redo());

    history.record(step("a", "ax"), Join::New);

    assert!(!history.can_redo(), "the undone branch survived a new edit");
    let mut current = doc("ax");
    assert_eq!(undo(&mut history, &mut current), Some(doc("a")));
}

#[test]
fn the_history_is_bounded() {
    let mut history = EditHistory::new();
    for n in 0..EditHistory::DEPTH + 50 {
        history.record(step(&n.to_string(), &(n + 1).to_string()), Join::New);
    }

    let mut current = doc(&(EditHistory::DEPTH + 50).to_string());
    let mut walked = 0;
    while history.undo(&mut current).is_some() {
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
    history.record(step("a", "ab"), Join::New);
    let mut current = doc("ab");
    history.undo(&mut current);
    history.clear();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
}
