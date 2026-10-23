//! The picker in the real window. specs/009-focus-macos.

/// FR-021 in GTK.
pub fn s_then_2_snoozes() {
    let window = open();
    window.press("s 2");
    assert_eq!(window.toast(), "Snoozed until tomorrow");
}

pub fn skips_without_a_display() {
    if gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    assert!(open().is_visible());
}
