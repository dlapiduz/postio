//! Rules (fixture). specs/007-postio-focus.

/// SC-002.
#[test]
fn the_label_is_fixed() {
    assert_eq!(A11Y_QUERY, "Search query");
}

#[test]
fn the_widths_add_up() {
    assert_eq!(LIST_MIN + PANE_MIN, 980);
    assert_eq!(Placement::default(), Placement::Dialog);
}

#[test]
fn a_computed_value_meets_a_constant() {
    let width = layout(1200).list;
    assert_eq!(width, LIST_MIN);
}

#[test]
fn dispatch_calls_the_backend_once() {
    let backend = Mock::new();
    dispatch(&backend);
    assert_eq!(backend.calls().len(), 1);
    assert_eq!(backend.call_count, 1);
}

#[test]
fn dispatch_moves_the_message() {
    let backend = Mock::new();
    dispatch(&backend);
    assert_eq!(backend.calls().len(), 1);
    assert_eq!(store.folder_of(id), "Archive");
}

#[test]
fn a_count_in_the_words_is_not_a_mock_count() {
    let calls = log();
    assert_eq!(line(&calls[1]), "answered 2 times");
}
