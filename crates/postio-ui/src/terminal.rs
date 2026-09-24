//! What a terminal frontend needs to know that a windowing toolkit does not.
//!
//! A terminal is a byte stream, and two things follow from that. Some chords
//! a desktop delivers cannot arrive at all -- `ctrl+Return` is the same byte
//! as `Return` in a terminal that does not speak the kitty keyboard protocol
//! -- and text from a message can carry control sequences the terminal would
//! obey. This module answers the first; the second is `SafeText`
//! (`specs/005-tui-frontend` research R4, R5).

use crate::keymap::{Chord, Key, Modifiers};

/// Whether a terminal *without* the kitty keyboard protocol can deliver
/// `chord` as itself.
///
/// A legacy terminal sends `ctrl` plus a letter as one control byte, so
/// `ctrl+i` is `Tab`, `ctrl+m` is `Return`, `ctrl+h` is `BackSpace` and `ctrl+j`
/// a line feed; `ctrl+shift` plus a letter is the same byte as `ctrl` plus
/// that letter; and `ctrl` or `shift` with `Return`, or `shift+space`, arrive
/// without their modifier. `super` never reaches a terminal at all. Named keys
/// with modifiers (`shift+Up`, `ctrl+Left`, `shift+Tab`, the function keys)
/// have xterm encodings every modern terminal sends.
pub fn legacy_deliverable(chord: &Chord) -> bool {
    let modifiers = chord.modifiers;
    if modifiers.contains(Modifiers::SUPER) {
        return false;
    }
    let ctrl = modifiers.contains(Modifiers::CTRL);
    let shift = modifiers.contains(Modifiers::SHIFT);
    match &chord.key {
        // `Chord::new` folds shift into the character, so `ctrl+shift+x`
        // arrives here as `ctrl` + `X`: the same byte as `ctrl+x`.
        Key::Char(character) if ctrl => {
            character.is_ascii_lowercase() && !matches!(character, 'i' | 'm' | 'h' | 'j')
        }
        Key::Char(_) => true,
        Key::Named(name) => match *name {
            "Return" => !ctrl && !shift,
            "Space" => !shift,
            "Tab" | "BackSpace" => !ctrl,
            _ => true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        text.parse().expect("test chord parses")
    }

    #[test]
    fn plain_keys_and_letters_with_ctrl_arrive() {
        for text in ["a", "A", "?", "Return", "Escape", "F5", "ctrl+k", "ctrl+a", "alt+x"] {
            assert!(legacy_deliverable(&chord(text)), "{text} should arrive");
        }
    }

    #[test]
    fn named_keys_with_modifiers_arrive() {
        for text in ["shift+Up", "shift+Down", "shift+Tab", "ctrl+Left", "shift+F5"] {
            assert!(legacy_deliverable(&chord(text)), "{text} should arrive");
        }
    }

    #[test]
    fn chords_a_legacy_terminal_folds_into_another_key_do_not() {
        for text in [
            "ctrl+Return",
            "shift+Return",
            "ctrl+shift+x",
            "ctrl+shift+Return",
            "ctrl+i",
            "ctrl+m",
            "ctrl+h",
            "ctrl+j",
            "ctrl+comma",
            "ctrl+7",
            "ctrl+shift+7",
            "shift+space",
            "super+k",
        ] {
            assert!(!legacy_deliverable(&chord(text)), "{text} should not arrive");
        }
    }
}

#[cfg(test)]
mod registry {
    use super::legacy_deliverable;
    use crate::keymap::Binding;
    use postio_config::{keys::expand_mod, paths::Platform};

    /// Every command must have at least one binding a legacy terminal can
    /// deliver, or the terminal frontend could only reach it through the
    /// palette -- which Principle II does not accept as reachable
    /// (`specs/005-tui-frontend` SC-001, research R4).
    #[test]
    fn every_command_has_a_binding_a_legacy_terminal_delivers() {
        let unreachable: Vec<String> = postio_core::registry::all()
            .filter(|spec| {
                !spec.bindings().any(|binding| {
                    expand_mod(binding, Platform::Freedesktop)
                        .parse::<Binding>()
                        .is_ok_and(|parsed| parsed.chords().iter().all(legacy_deliverable))
                })
            })
            .map(|spec| format!("{} ({})", spec.id.as_str(), spec.default_binding))
            .collect();
        assert!(
            unreachable.is_empty(),
            "{} command(s) have no binding a legacy terminal can deliver:\n{}",
            unreachable.len(),
            unreachable.join("\n")
        );
    }
}
