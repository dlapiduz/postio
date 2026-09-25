//! Key hints: the key a surface names beside the thing it does.
//!
//! docs/PRODUCT.md §8 says the key hints are *derived* from the registry, the
//! same one table the keymap, the palette and the cheat sheet read. That was
//! true of the focused row and the reader's bar, and not of a dozen other
//! places that typed the key in -- `"c"` beside Compose, `"H"` beside Render
//! once, `"C-⇧-A"` beside Attach another -- and went on saying it after a
//! `[keys]` rebind moved the command somewhere else. A hint that lies is worse
//! than none (#828).
//!
//! So a hint is built from a [`Keymap`] and a [`CommandId`], and a command
//! with no binding produces no hint rather than a blank one. The only other
//! way to make one is [`fixed`], for a key that is not a command at all --
//! `Tab` between a form's fields, `Return` on a screen's one button -- and it
//! asks for the reason, because "it is not a command" is a claim somebody
//! should be able to check.
//!
//! The key is spelled the way the registry spells it (`ctrl+Return`, `e`,
//! `Escape`), which is what the palette, the cheat sheet and
//! `docs/keybindings.md` already show: one notation, so a key learned on one
//! surface is recognised on the next.
//!
//! No toolkit here. `postio-gtk`'s `widgets::keyhint` draws these; a second
//! frontend draws the same ones.

use postio_core::{CommandId, Keymap};

/// One key, and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    /// The key, as the registry spells it: `e`, `ctrl+Return`, `j/k`.
    pub key: String,
    /// What it does, in the surface's words: `reply`, `Compose`.
    pub label: String,
}

/// The hint for `command`, or `None` when nothing is bound to it.
///
/// `None` is the case where the user cleared a binding or lost it to a
/// collision. The caller draws nothing -- a blank cap would read as a key
/// that exists and does nothing.
pub fn hint(keymap: &Keymap, command: CommandId, label: &str) -> Option<Hint> {
    keymap.binding(command).map(|key| Hint {
        key: key.to_owned(),
        label: label.to_owned(),
    })
}

/// As [`hint`], naming `preferred` when it is one of `command`'s keys.
///
/// For a surface the canvas draws with a command's *alternate*: the list's
/// failure plates say `R` for Refresh, whose primary key is `F5`. The
/// preference is only honoured while the keymap actually binds it, so a
/// rebind that takes `R` away falls back to the primary rather than naming a
/// key that no longer does this.
pub fn hint_as(keymap: &Keymap, command: CommandId, preferred: &str, label: &str) -> Option<Hint> {
    let bindings = keymap.bindings(command);
    let key = bindings
        .iter()
        .find(|binding| *binding == preferred)
        .or_else(|| bindings.first())?;
    Some(Hint {
        key: key.clone(),
        label: label.to_owned(),
    })
}

/// Just the key for `command`, for a control that draws its own label.
pub fn key(keymap: &Keymap, command: CommandId) -> Option<String> {
    keymap.binding(command).map(str::to_owned)
}

/// Two commands that are one idea -- next and previous -- as one hint:
/// `j/k walk`.
///
/// Either half alone still teaches something, so a pair with one side
/// unbound shows the side that is bound rather than nothing.
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

/// A hint for a key that is not a command.
///
/// `because` says why no command could carry it -- `Tab` moving between a
/// form's fields is the toolkit's, `Return` pressing a screen's only button
/// is that button's default -- and it is required so the claim is written
/// where it is made. `scripts/checks/check-key-hints-are-derived.py` counts
/// the callers against an allowlist with the same reasons.
pub fn fixed(key: &str, label: &str, because: &'static str) -> Hint {
    debug_assert!(!because.is_empty(), "a fixed key hint says why");
    Hint {
        key: key.to_owned(),
        label: label.to_owned(),
    }
}

/// A row of hints as one line of text: `j/k walk · Return open · s save`.
///
/// The key first, then what it does, because the key is the thing being
/// taught and the eye finds it at the start of each clause.
pub fn line<'a>(hints: impl IntoIterator<Item = &'a Hint>) -> String {
    hints
        .into_iter()
        .map(|hint| format!("{} {}", hint.key, hint.label))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // An override outranks a default, so giving `c` to archive leaves
        // Compose with no key at all -- palette-only.
        let keymap = rebound(CommandId::Archive, "c");
        assert_eq!(hint(&keymap, CommandId::Compose, "Compose"), None);
    }

    #[test]
    fn a_preferred_key_is_named_only_while_it_is_bound() {
        let retry = |keymap: &Keymap| {
            hint_as(keymap, CommandId::Refresh, "R", "Retry now").map(|hint| hint.key)
        };
        assert_eq!(retry(Keymap::defaults()).as_deref(), Some("R"));
        // Giving `R` to archive takes it from Refresh; the plate falls back
        // to the key Refresh still has.
        assert_eq!(
            retry(&rebound(CommandId::Archive, "R")).as_deref(),
            Some("F5")
        );
    }

    #[test]
    fn a_pair_is_one_clause() {
        let walk = pair(
            Keymap::defaults(),
            CommandId::NextPart,
            CommandId::PrevPart,
            "walk",
        );
        assert_eq!(walk.map(|h| h.key).as_deref(), Some("j/k"));
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
