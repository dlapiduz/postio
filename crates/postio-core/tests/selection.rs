//! What an action will hit, and how the keyboard changes it.
//!
//! The constraint that shapes all of this: docs/PRODUCT.md §18 says a mailbox is never
//! loaded into memory, so "select all" in a 100,000-message folder must not be
//! expressible only as 100,000 ids. It is a predicate instead, and the storage
//! layer resolves it in one statement when an action finally lands.

use postio_config::paths::Platform;
use postio_core::state::Selection;
use postio_core::{AppState, CommandId, Context, Event, registry};
use postio_model::MessageId;

fn ids(count: i64) -> Vec<MessageId> {
    (1..=count).map(MessageId::new).collect()
}

#[test]
fn nothing_is_selected_to_begin_with() {
    let state = AppState::new();
    assert!(state.selection().is_empty());
    assert_eq!(state.selection().ids(), Some(&[][..]));
}

#[test]
fn picking_rows_selects_exactly_those_rows() {
    let mut state = AppState::new();
    state.select(ids(3), Some(MessageId::new(2)));

    assert!(!state.selection().is_empty());
    assert_eq!(state.selection().ids(), Some(&ids(3)[..]));
    assert!(state.selection().contains(MessageId::new(2)));
    assert!(!state.selection().contains(MessageId::new(9)));
    assert_eq!(state.focus(), Some(MessageId::new(2)));
}

#[test]
fn toggling_adds_a_row_and_toggling_again_takes_it_away() {
    let mut state = AppState::new();
    state.toggle_selection(MessageId::new(4));
    assert_eq!(state.selection().ids(), Some(&[MessageId::new(4)][..]));

    state.toggle_selection(MessageId::new(4));
    assert!(!state.selection().contains(MessageId::new(4)));
    assert!(state.selection().is_empty());
}

#[test]
fn extending_adds_the_row_and_takes_the_focus_with_it() {
    let mut state = AppState::new();
    state.select(vec![MessageId::new(1)], Some(MessageId::new(1)));
    state.extend_selection_to(MessageId::new(2));

    assert!(state.selection().contains(MessageId::new(1)));
    assert!(state.selection().contains(MessageId::new(2)));
    assert_eq!(
        state.focus(),
        Some(MessageId::new(2)),
        "extending moves the keyboard with the selection"
    );
}

#[test]
fn select_all_is_a_predicate_rather_than_a_hundred_thousand_ids() {
    // The whole point. If this ever becomes a `Vec`, the windowed list model
    // is defeated by one keystroke.
    let mut state = AppState::new();
    state.select_all();

    assert!(!state.selection().is_empty());
    assert_eq!(
        state.selection().ids(),
        None,
        "everything-in-the-mailbox cannot be named as a list of ids"
    );
    assert!(state.selection().contains(MessageId::new(87_412)));
    assert!(state.selection().is_everything());
}

#[test]
fn deselecting_inside_select_all_stays_a_predicate() {
    let mut state = AppState::new();
    state.select_all();
    state.toggle_selection(MessageId::new(7));

    assert!(state.selection().is_everything());
    assert!(
        !state.selection().contains(MessageId::new(7)),
        "a row taken out of everything is out of the selection"
    );
    assert!(state.selection().contains(MessageId::new(8)));

    // And putting it back is the same gesture again.
    state.toggle_selection(MessageId::new(7));
    assert!(state.selection().contains(MessageId::new(7)));
    assert_eq!(state.selection(), &Selection::Everything { except: vec![] });
}

#[test]
fn clearing_gives_up_the_predicate_too() {
    let mut state = AppState::new();
    state.select_all();
    state.clear_selection();

    assert!(state.selection().is_empty());
    assert!(!state.selection().is_everything());
}

#[test]
fn every_selection_change_is_announced() {
    let mut state = AppState::new();
    let events = state.select_all();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::SelectionChanged { .. })),
        "nothing told the toolbar its count moved: {events:?}"
    );

    // And an idempotent change says nothing, so a repaint is never wasted.
    let again = state.select_all();
    assert!(
        !again
            .iter()
            .any(|event| matches!(event, Event::SelectionChanged { .. })),
        "selecting all twice announced a change that did not happen"
    );
}

#[test]
fn the_four_selection_keys_are_in_the_registry_and_overridable() {
    // All four go through the registry so `[keys]` can move them, and so the
    // palette and the `?` sheet print them beside everything else.
    for (id, binding) in [
        (CommandId::ToggleSelection, "x"),
        (CommandId::ExtendSelectionDown, "J"),
        (CommandId::ExtendSelectionUp, "K"),
        (CommandId::SelectAll, "ctrl+a"),
    ] {
        let spec = registry::get(id);
        assert_eq!(
            postio_config::keys::expand_mod(spec.default_binding, Platform::Freedesktop),
            binding,
            "`{id}`"
        );
        assert_eq!(
            registry::lookup_binding_on(Context::List, binding, Platform::Freedesktop)
                .map(|spec| spec.id),
            Some(id),
            "`{binding}` should resolve to `{id}` in the message list"
        );
        assert!(
            !spec.destructive,
            "`{id}` changes no durable state, so it is not destructive"
        );
        assert!(
            registry::for_context(Context::List).any(|candidate| candidate.id == id),
            "`{id}` is missing from the message list's palette"
        );
    }
}

#[test]
fn an_explicit_binding_outranks_a_built_in_default_that_wants_the_same_key() {
    // `x` is `toggle_selection`'s default. Someone who writes
    // `archive = "x"` has said what they want, and a default is only ever a
    // suggestion — so the override takes the key and the default gives it up.
    // Without this rule, adding a command with a popular default silently
    // steals a key somebody had already asked for, and which command wins
    // depends on the order of a table they have never seen.
    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_string(), "x".to_string());
    let keymap = postio_core::Keymap::resolve(&overrides);

    assert_eq!(keymap.binding(CommandId::Archive), Some("x"));
    assert_ne!(
        keymap.binding(CommandId::ToggleSelection),
        Some("x"),
        "the default should have given the key up"
    );
    assert!(
        keymap
            .problems()
            .iter()
            .any(|problem| problem.contains("toggle_selection")),
        "and said so: {:?}",
        keymap.problems()
    );
}
