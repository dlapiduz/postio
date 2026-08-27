//! The command registry, as the frontend sees it.
//!
//! `PRODUCT.md` §8: *a command that is not in the registry does not exist* —
//! not merely unbound, but absent from every way a user could discover it. The
//! corollary for a second frontend is that the registry has to *reach* it, or
//! the macOS palette and cheat sheet become a hand-maintained list that drifts
//! from the Linux one the first time either is edited.

use postio_ffi::{Session, SessionOptions, UiRecovery};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn every_registry_row_crosses() {
    let session = session();
    let crossed = session.commands();

    // Against the registry's own count, so a command added on the Rust side
    // cannot quietly fail to reach the frontend. A hardcoded number here
    // would go stale and start asserting the past.
    assert_eq!(
        crossed.len(),
        postio_core::registry::all().count(),
        "the boundary dropped commands on the way across"
    );
    session.shutdown();
}

#[test]
fn a_row_carries_what_a_palette_and_a_menu_need() {
    let session = session();
    let crossed = session.commands();

    let archive = crossed
        .iter()
        .find(|spec| spec.id == "archive")
        .expect("`archive` is in the registry");

    assert!(!archive.title.is_empty(), "a palette row with no title");
    assert!(
        !archive.default_binding.is_empty(),
        "no default binding, so the cheat sheet has nothing to print"
    );
    assert!(
        !archive.contexts.is_empty(),
        "no contexts, so the palette cannot tell whether to offer it"
    );
    session.shutdown();
}

#[test]
fn destructive_commands_carry_their_recovery() {
    // `PRODUCT.md`: destructive operations are confirmed or undoable, and the
    // registry is where that is machine-checked. A frontend that cannot see
    // `destructive` cannot confirm, and one that cannot see `recovery` cannot
    // tell "ask first" from "offer an undo" -- so both have to cross, and
    // `Recovery::None` on a destructive command must be impossible here as it
    // is on the Rust side.
    let session = session();
    for spec in session.commands() {
        if spec.destructive {
            assert_ne!(
                spec.recovery,
                UiRecovery::None,
                "{} is destructive with no recovery, so no frontend could \
                 honour the invariant",
                spec.id
            );
        }
    }
    session.shutdown();
}

#[test]
fn ids_are_the_strings_config_uses() {
    // The whole reason commands cross as strings: `[keys]` in `config.toml`
    // names these exact ids, so a binding, a palette row and a log line all
    // spell a command the same way -- and a new command reaches the macOS UI
    // with no Swift change.
    let session = session();
    let crossed = session.commands();

    for spec in postio_core::registry::all() {
        assert!(
            crossed.iter().any(|row| row.id == spec.id.as_str()),
            "{} did not cross under the id `[keys]` uses",
            spec.id.as_str()
        );
    }
    session.shutdown();
}
