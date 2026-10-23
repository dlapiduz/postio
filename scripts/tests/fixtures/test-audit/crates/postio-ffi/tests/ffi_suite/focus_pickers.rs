//! The Mac gets the picker. specs/009-focus-macos.

/// FR-021, on the Mac.
#[test]
fn a_number_chooses_on_the_mac() {
    let shown = bridge().press("2");
    assert_eq!(shown.title(), "Snoozed until tomorrow");
}

#[test]
fn opening_succeeds() {
    let result = bridge().open();
    assert!(result.is_ok());
}

#[test]
fn opening_yields_something() {
    let result = bridge().open();
    assert!(result.is_some(), "opened");
}

#[test]
fn opening_returns_the_picker() {
    let result = bridge().open();
    assert!(result.is_ok());
    assert_eq!(result.unwrap().name(), "snooze");
}
