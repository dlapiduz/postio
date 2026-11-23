//! Key hints: the key a surface names beside what it does, built from a `Keymap` and a `CommandId`
//! so a rebind cannot leave a stale hint (#828).
//! A command with no binding produces no hint. `fixed` is the only other way, for a key that is not
//! a command, and requires a reason.

use postio_core::{CommandId, Keymap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub key: String,
    pub label: String,
}

/// `None` when the binding is cleared or lost to a collision; the caller draws nothing.
pub fn hint(keymap: &Keymap, command: CommandId, label: &str) -> Option<Hint> {
    keymap.binding(command).map(|key| Hint {
        key: key.to_owned(),
        label: label.to_owned(),
    })
}

/// The `aria-keyshortcuts` form: modifiers in full joined by `+`, `Return` is `Enter`, a capital
/// letter says its Shift. A sequence (`g i`) has no ARIA spelling and is left as is.
pub fn shortcut(key: &str) -> String {
    if key.contains(' ') {
        return key.to_owned();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut pieces = key.split('+').peekable();
    while let Some(piece) = pieces.next() {
        let last = pieces.peek().is_none();
        if !last {
            parts.push(
                match piece {
                    "ctrl" | "control" | "mod" => "Control",
                    "alt" => "Alt",
                    "shift" | "\u{21e7}" => "Shift",
                    "super" | "meta" | "cmd" => "Meta",
                    other => other,
                }
                .to_owned(),
            );
            continue;
        }
        let name = match piece {
            "Return" | "\u{21b5}" => "Enter".to_owned(),
            "Del" => "Delete".to_owned(),
            letter
                if letter.chars().count() == 1
                    && letter.chars().all(char::is_uppercase)
                    && !parts.iter().any(|part| part == "Shift") =>
            {
                parts.push("Shift".to_owned());
                letter.to_owned()
            }
            other => other.to_owned(),
        };
        parts.push(name);
    }
    parts.join("+")
}

/// A cap in a tight row: `Delete` is `Del`, `Return` and `shift` are `↵` and `⇧`; other names
/// keep the keymap's spelling. What is pressed and what a screen reader hears keep the binding's
/// names.
pub fn short(key: &str) -> String {
    if key.contains(' ') {
        return key.to_owned();
    }
    key.split('+')
        .map(|piece| match piece {
            "Delete" => "Del",
            "Return" => "\u{21b5}",
            "shift" => "\u{21e7}",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

pub fn key(keymap: &Keymap, command: CommandId) -> Option<String> {
    keymap.binding(command).map(str::to_owned)
}

/// A pair with one side unbound shows the side that is bound.
pub fn pair(keymap: &Keymap, first: CommandId, second: CommandId, label: &str) -> Option<Hint> {
    let key = match (keymap.binding(first), keymap.binding(second)) {
        (Some(a), Some(b)) => format!("{a}/{b}"),
        (Some(one), None) | (None, Some(one)) => one.to_owned(),
        (None, None) => return None,
    };
    Some(Hint {
        key,
        label: label.to_owned(),
    })
}

/// One cap only while the keys are one modifier and digits in order; a rebind that breaks it says
/// the first and last apart.
pub fn run(keymap: &Keymap, commands: &[CommandId], label: &str) -> Option<Hint> {
    let (first, last) = (commands.first()?, commands.last()?);
    let keys: Vec<Option<&str>> = commands.iter().map(|id| keymap.binding(*id)).collect();
    let modifier = keys.first().copied().flatten()?.strip_suffix('1');
    let in_order = commands.len() > 1
        && modifier.is_some_and(|modifier| {
            keys.iter().enumerate().all(|(at, key)| {
                key.and_then(|key| key.strip_prefix(modifier))
                    .is_some_and(|digit| digit == (at + 1).to_string())
            })
        });
    match modifier {
        Some(modifier) if in_order => Some(Hint {
            key: format!("{modifier}1\u{2013}{}", commands.len()),
            label: label.to_owned(),
        }),
        _ => pair(keymap, *first, *last, label),
    }
}

/// `because` says why no command could carry the key;
/// `scripts/checks/check-key-hints-are-derived.py` counts callers against an allowlist.
pub fn fixed(key: &str, label: &str, because: &'static str) -> Hint {
    debug_assert!(!because.is_empty(), "a fixed key hint says why");
    Hint {
        key: key.to_owned(),
        label: label.to_owned(),
    }
}

pub fn line<'a>(hints: impl IntoIterator<Item = &'a Hint>) -> String {
    hints
        .into_iter()
        .map(|hint| format!("{} {}", hint.key, hint.label))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_key_is_spelled_for_a_screen_reader_as_aria_keyshortcuts_spells_it() {
        assert_eq!(shortcut("e"), "e");
        assert_eq!(shortcut("ctrl+Return"), "Control+Enter");
        assert_eq!(shortcut("ctrl+shift+a"), "Control+Shift+a");
        assert_eq!(shortcut("alt+s"), "Alt+s");
        assert_eq!(shortcut("Escape"), "Escape");
        assert_eq!(shortcut("E"), "Shift+E");
        assert_eq!(shortcut("g i"), "g i");
        assert_eq!(shortcut("super+k"), "Meta+k");
    }
    use super::*;

    #[test]
    fn a_tight_cap_compacts_delete_return_and_shift_and_nothing_else() {
        assert_eq!(short("Delete"), "Del");
        assert_eq!(short("shift+Delete"), "\u{21e7}+Del");
        assert_eq!(short("ctrl+Return"), "ctrl+\u{21b5}");
        assert_eq!(short("ctrl+shift+Return"), "ctrl+\u{21e7}+\u{21b5}");
        assert_eq!(short("ctrl+shift+a"), "ctrl+\u{21e7}+a");
        assert_eq!(short("Return"), "\u{21b5}");
        for key in ["e", "E", "Escape", "ctrl+h", "g i", ".", "alt+s"] {
            assert_eq!(short(key), key);
        }
        assert_eq!(
            shortcut("Delete"),
            "Delete",
            "a screen reader hears the name"
        );
        for (key, heard) in [
            ("Delete", "Delete"),
            ("ctrl+Return", "Control+Enter"),
            ("ctrl+shift+Return", "Control+Shift+Enter"),
            ("ctrl+shift+a", "Control+Shift+a"),
        ] {
            assert_eq!(
                shortcut(&short(key)),
                heard,
                "a cap read back for its shortcut is named in full"
            );
        }
    }

    fn rebound(command: CommandId, key: &str) -> Keymap {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert(command.as_str().to_owned(), key.to_owned());
        Keymap::resolve(&overrides)
    }

    #[test]
    fn a_hint_says_the_key_the_registry_binds() {
        let hint = hint(Keymap::defaults(), CommandId::Compose, "Compose");
        assert_eq!(
            hint,
            Some(Hint {
                key: "c".into(),
                label: "Compose".into()
            })
        );
    }

    #[test]
    fn a_rebind_reaches_the_hint() {
        let keymap = rebound(CommandId::Compose, "w");
        assert_eq!(key(&keymap, CommandId::Compose).as_deref(), Some("w"));
    }

    #[test]
    fn an_unbound_command_has_no_hint() {
        // An override outranks a default, so Snooze (one key, no alternate) loses its key; Compose
        // has `mod+n` to fall back on.
        let keymap = rebound(CommandId::Compose, "b");
        assert_eq!(hint(&keymap, CommandId::Snooze, "Snooze"), None);
    }

    #[test]
    fn a_pair_is_one_clause() {
        let walk = pair(
            Keymap::defaults(),
            CommandId::NextMessage,
            CommandId::PrevMessage,
            "walk",
        );
        assert_eq!(walk.map(|h| h.key).as_deref(), Some("j/k"));
    }

    #[test]
    fn a_numbered_run_is_one_range_and_a_broken_one_its_ends() {
        let saved = [
            CommandId::SavedSearch1,
            CommandId::SavedSearch2,
            CommandId::SavedSearch3,
            CommandId::SavedSearch4,
        ];
        let defaults = run(Keymap::defaults(), &saved, "saved");
        assert_eq!(defaults.map(|h| h.key).as_deref(), Some("alt+1\u{2013}4"));
        let keymap = rebound(CommandId::SavedSearch3, "alt+9");
        let broken = run(&keymap, &saved, "saved");
        assert_eq!(broken.map(|h| h.key).as_deref(), Some("alt+1/alt+4"));
        assert_eq!(run(Keymap::defaults(), &[], "nothing"), None);
    }

    #[test]
    fn a_line_puts_each_key_before_what_it_does() {
        let hints = [
            fixed("Tab", "refine", "the panel's own focus order"),
            hint(Keymap::defaults(), CommandId::OpenMessage, "open").unwrap(),
        ];
        assert_eq!(line(&hints), "Tab refine · Return open");
    }
}
