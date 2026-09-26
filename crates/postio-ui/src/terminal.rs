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

/// The binding to show for `command` in a terminal: its first binding the
/// terminal can deliver.
///
/// With the kitty keyboard protocol (`enhanced`) every chord arrives, so it is
/// the binding in force, `[keys]` override and all. Without it, it is the
/// first binding every chord of which a legacy terminal can send -- the
/// alternate that exists for exactly this -- so a palette row or cheat-sheet
/// line never shows a key that would do nothing when pressed.
pub fn deliverable_binding(
    keymap: &postio_core::Keymap,
    command: impl Into<postio_core::ActionId>,
    enhanced: bool,
) -> Option<String> {
    let bindings = keymap.bindings(command);
    if enhanced {
        return bindings.first().cloned();
    }
    bindings
        .iter()
        .find(|binding| {
            binding
                .parse::<crate::keymap::Binding>()
                .is_ok_and(|parsed| parsed.chords().iter().all(legacy_deliverable))
        })
        .cloned()
}

/// Text from a message, made safe to put in front of a terminal.
///
/// Mail is attacker-controlled, and a terminal obeys what it is sent: an
/// `ESC ] 0 ;` in a subject retitles the window, an `ESC [ 2 J` clears the
/// screen, and a right-to-left override makes a filename read as something
/// it is not. This is the terminal's `<script>` (research R5). The only way
/// to build one is [`SafeText::new`], which replaces every C0 control except
/// newline and tab, `DEL`, every C1 control, and the bidirectional overrides
/// and isolates with a visible stand-in, so what was there is still seen but
/// never obeyed. A frontend that takes message text any other way has left
/// this boundary, which is why a terminal view accepts `SafeText` and not
/// `&str`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SafeText(String);

impl SafeText {
    /// `text`, with everything a terminal would act on made visible instead.
    pub fn new(text: &str) -> SafeText {
        SafeText(text.chars().map(stand_in).collect())
    }

    /// The safe text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a terminal is shown in place of `c`.
///
/// A C0 control becomes its Unicode control picture (`ESC` is `␛`), so a
/// hostile subject still reads as what it tried to do. `DEL` is `␡`. C1
/// controls and bidirectional overrides have no pictures and become `�`.
fn stand_in(c: char) -> char {
    match c {
        '\n' | '\t' => c,
        '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{fffd}'),
        '\u{7f}' => '\u{2421}',
        '\u{80}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => '\u{fffd}',
        _ => c,
    }
}

impl std::fmt::Display for SafeText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod safe_text {
    use super::SafeText;

    #[test]
    fn a_title_or_clear_sequence_is_seen_not_obeyed() {
        let safe = SafeText::new("Invoice\u{1b}]0;pwned\u{7}\u{1b}[2J due");
        assert!(!safe.as_str().contains('\u{1b}'), "{safe:?}");
        assert!(!safe.as_str().contains('\u{7}'), "{safe:?}");
        assert!(
            safe.as_str().contains("]0;pwned"),
            "what was there is still visible: {safe:?}"
        );
        assert!(safe.as_str().starts_with("Invoice") && safe.as_str().ends_with("due"));
    }

    #[test]
    fn c1_controls_and_delete_are_replaced_too() {
        // U+009B is a one-character CSI in terminals that honour 8-bit C1.
        let safe = SafeText::new("a\u{9b}2Jb\u{7f}c\u{85}d");
        for c in ['\u{9b}', '\u{7f}', '\u{85}'] {
            assert!(!safe.as_str().contains(c), "{c:?} survived: {safe:?}");
        }
    }

    #[test]
    fn bidirectional_overrides_cannot_disguise_a_name() {
        // "invoice\u{202e}fdp.exe" displays as "invoiceexe.pdf".
        let safe = SafeText::new("invoice\u{202e}fdp.exe \u{2066}x\u{2069}");
        for c in [
            '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}',
        ] {
            assert!(!safe.as_str().contains(c), "{c:?} survived: {safe:?}");
        }
    }

    #[test]
    fn ordinary_text_newlines_and_tabs_pass_untouched() {
        let text = "Grüße, 山田さん 👋\n\tNext line — “quoted”";
        assert_eq!(SafeText::new(text).as_str(), text);
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
        for text in [
            "a", "A", "?", "Return", "Escape", "F5", "ctrl+k", "ctrl+a", "alt+x",
        ] {
            assert!(legacy_deliverable(&chord(text)), "{text} should arrive");
        }
    }

    #[test]
    fn named_keys_with_modifiers_arrive() {
        for text in [
            "shift+Up",
            "shift+Down",
            "shift+Tab",
            "ctrl+Left",
            "shift+F5",
        ] {
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
            assert!(
                !legacy_deliverable(&chord(text)),
                "{text} should not arrive"
            );
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
    fn a_legacy_terminal_is_shown_the_alternate_it_can_send() {
        let keymap = postio_core::Keymap::resolve(&Default::default());
        let mark_sent = postio_core::CommandId::MarkSent;
        assert_eq!(
            super::deliverable_binding(&keymap, mark_sent, true).as_deref(),
            Some("ctrl+shift+m")
        );
        assert_eq!(
            super::deliverable_binding(&keymap, mark_sent, false).as_deref(),
            Some("alt+m")
        );
        assert_eq!(
            super::deliverable_binding(&keymap, postio_core::CommandId::Archive, false).as_deref(),
            Some("a")
        );
    }

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
