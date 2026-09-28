//! Focus's key map is grouped, and every key Focus offers is in a group
//! (specs/007-postio-focus T031, research R4).
//!
//! The key map (screen 20) draws its keys in the groups the design names;
//! the groups table in `postio_ui::keymap_sheet` says which group a command
//! sits in, and the registry says its key. A command Focus offers with no
//! group would be a key the key map never teaches -- which is the failure
//! the key map exists to prevent, so the table is enumerated here against
//! the registry, as the one keymap is (SC-015).

use postio_core::{Availability, CommandId, Frontend, Scope, registry};
use postio_model::AccountId;
use postio_ui::keymap_sheet::{Group, KEY_MAP_CONTEXTS, group};

/// Focus with mail open in one account: the state in which it offers the
/// most.
fn focus() -> Availability {
    Availability {
        frontend: Frontend::Focus,
        ..Availability::open(Scope::Account(AccountId::new(1)))
    }
}

/// Every command Focus offers where its key map applies.
fn offered_by_focus() -> Vec<CommandId> {
    let mut offered: Vec<CommandId> = KEY_MAP_CONTEXTS
        .iter()
        .flat_map(|context| registry::reachable_in(*context, focus()))
        .filter_map(|action| action.id.builtin())
        .collect();
    offered.sort();
    offered.dedup();
    offered
}

#[test]
fn every_command_focus_offers_has_a_group() {
    let missing: Vec<CommandId> = offered_by_focus()
        .into_iter()
        .filter(|id| group(*id).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "Focus offers these and its key map would never teach them: {missing:?}"
    );
}

#[test]
fn no_group_holds_a_command_focus_does_not_offer() {
    // The other way round: a row for a command Focus never offers is a key
    // the key map teaches and the app does not have.
    let offered = offered_by_focus();
    let stray: Vec<CommandId> = CommandId::ALL
        .iter()
        .copied()
        .filter(|id| group(*id).is_some() && !offered.contains(id))
        .collect();
    assert!(
        stray.is_empty(),
        "these have a group and Focus does not offer them: {stray:?}"
    );
}

#[test]
fn the_groups_are_the_ones_the_design_draws() {
    let titles: Vec<&str> = Group::ALL.iter().map(|group| group.title()).collect();
    assert_eq!(
        titles,
        [
            "Move and select",
            "Open",
            "Act (row or selection)",
            "Invites",
            "Go and find",
            "In search",
            "Digests and filtering",
            "Obsidian",
        ]
    );
}
