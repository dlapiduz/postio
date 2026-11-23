//! A GDK key press as the keymap resolver reads it; both desktop apps share it (ADR 0043).

use gtk::gdk;
use postio_ui::keymap::{Chord, Modifiers};

/// `None` for a key with no name (a dead key), which the caller should let propagate.
pub fn chord(keyval: gdk::Key, state: gdk::ModifierType) -> Option<Chord> {
    let mut modifiers = Modifiers::NONE;
    for (mask, modifier) in [
        (gdk::ModifierType::CONTROL_MASK, Modifiers::CTRL),
        (gdk::ModifierType::ALT_MASK, Modifiers::ALT),
        (gdk::ModifierType::SHIFT_MASK, Modifiers::SHIFT),
        (gdk::ModifierType::SUPER_MASK, Modifiers::SUPER),
    ] {
        if state.contains(mask) {
            modifiers = modifiers.with(modifier);
        }
    }
    let name = keyval.name();
    Chord::from_platform_key(keyval.to_unicode(), name.as_deref(), modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_event_is_the_chord_its_binding_is_written_as() {
        for (name, state, expected) in [
            ("a", gdk::ModifierType::empty(), "a"),
            ("J", gdk::ModifierType::SHIFT_MASK, "J"),
            ("z", gdk::ModifierType::CONTROL_MASK, "ctrl+z"),
            ("exclam", gdk::ModifierType::SHIFT_MASK, "!"),
            ("question", gdk::ModifierType::SHIFT_MASK, "?"),
            ("Escape", gdk::ModifierType::empty(), "Escape"),
            ("Delete", gdk::ModifierType::empty(), "Delete"),
            ("space", gdk::ModifierType::empty(), "Space"),
        ] {
            let pressed = chord(gdk::Key::from_name(name).expect("a keysym"), state)
                .unwrap_or_else(|| panic!("{name} made no chord"));
            let written: Chord = expected.parse().expect("a chord");
            assert_eq!(pressed, written, "{name} with {state:?}");
        }
    }
}
