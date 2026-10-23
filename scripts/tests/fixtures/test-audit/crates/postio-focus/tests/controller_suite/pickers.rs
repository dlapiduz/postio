//! The snooze picker's branches. specs/009-focus-macos, #1234.

/// FR-021: a number chooses.
#[test]
fn a_number_chooses() {
    let chosen = choose(2);
    assert_eq!(chosen, "tomorrow");
}
