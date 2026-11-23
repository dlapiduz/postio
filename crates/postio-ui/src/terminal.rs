//! What a terminal frontend needs that a windowing toolkit does not: which chords a legacy terminal
//! can deliver, and `SafeText`.

use crate::keymap::{Chord, Key, Modifiers};

/// A legacy terminal sends ctrl plus a letter as one control byte (`ctrl+i` is Tab, `ctrl+m`
/// Return) and drops the modifier from `ctrl+Return` and `shift+space`; `super` never arrives.
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
            name if name.starts_with("KP_") => false,
            "Return" => !ctrl && !shift,
            "Space" => !shift,
            "Tab" | "BackSpace" => !ctrl,
            _ => true,
        },
    }
}

/// With the kitty keyboard protocol (`enhanced`) every chord arrives; without it, the first binding
/// a legacy terminal can send, so no hint shows a key that does nothing.
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

/// Mail is attacker-controlled and a terminal obeys what it is sent (`ESC ] 0 ;` retitles the
/// window, a right-to-left override disguises a filename).
/// `SafeText::new` is the only constructor and replaces controls and bidi overrides with visible
/// stand-ins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SafeText(String);

impl SafeText {
    pub fn new(text: &str) -> SafeText {
        SafeText(text.chars().map(stand_in).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// C0 controls become their control pictures, `DEL` is `␡`, C1 controls and bidi overrides become
/// `�`.
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
        let safe = SafeText::new("a\u{9b}2Jb\u{7f}c\u{85}d");
        for c in ['\u{9b}', '\u{7f}', '\u{85}'] {
            assert!(!safe.as_str().contains(c), "{c:?} survived: {safe:?}");
        }
    }

    #[test]
    fn bidirectional_overrides_cannot_disguise_a_name() {
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

    #[test]
    fn a_legacy_terminal_is_shown_the_alternate_it_can_send() {
        let keymap = postio_core::Keymap::resolve_on(&Default::default(), Platform::Freedesktop);
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
            Some("e")
        );
    }

    #[test]
    fn every_command_has_a_binding_a_legacy_terminal_delivers() {
        let terminal = postio_core::Availability {
            frontend: postio_core::Frontend::Terminal,
            ..postio_core::Availability::open(postio_core::Scope::Unified)
        };
        let unreachable: Vec<String> = postio_core::registry::all()
            .filter(|spec| spec.requires.met_by(terminal))
            .filter(|spec| postio_core::registry::offered_on(spec.id.into(), Platform::Freedesktop))
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

#[cfg(test)]
mod safe_text_display_tests {
    use super::SafeText;

    #[test]
    fn it_displays_its_neutralised_text() {
        assert_eq!(SafeText::new("plain").to_string(), "plain");
        assert_eq!(SafeText::new("a\u{7f}b").to_string(), "a\u{2421}b");
    }
}
