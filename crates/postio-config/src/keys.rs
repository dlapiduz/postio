//! The file holds overrides only, as does `KeyBindings`: the defaults live in `postio-core`'s
//! command registry, which this crate cannot read (core depends on config), so ask
//! `postio_core::config::Keymap` (#1227). Binding syntax is untyped here, so a binding for an
//! unknown command is preserved.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::paths::Platform;

/// GTK spells the last two `Super`/`Meta`; both are accepted.
pub const MODIFIERS: &[&str] = &[
    "mod", "ctrl", "control", "cmd", "command", "alt", "shift", "super", "meta",
];

/// GDK key names, spelled case-insensitively here because people type `escape`.
pub const KEY_NAMES: &[&str] = &[
    "return",
    "enter",
    "escape",
    "esc",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "page_up",
    "page_down",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
    "menu",
    "kp_add",
    "kp_subtract",
    "kp_0",
    "exclam",
    "plus",
    "minus",
    "equal",
    "slash",
    "backslash",
    "question",
    "comma",
    "period",
    "semicolon",
    "colon",
    "asterisk",
    "underscore",
    "less",
    "greater",
    "bracketleft",
    "bracketright",
    "grave",
    "apostrophe",
    "quotedbl",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

/// Explain, in prose, why a binding cannot be used — or `None` if it is fine.
///
/// The syntax is the one the design canvas uses: a chord is a key with optional
/// `+`-joined modifiers (`ctrl+k`), and a sequence is chords separated by
/// spaces (`g s`). The message is written to be pasted straight into the
/// settings validity line.
///
/// ```
/// # use postio_config::keys::binding_problem;
/// assert_eq!(binding_problem("ctrl+k"), None);
/// assert_eq!(binding_problem("g s"), None);
/// assert!(binding_problem("ctrl+").is_some());
/// ```
pub fn binding_problem(binding: &str) -> Option<String> {
    if binding.trim().is_empty() {
        return Some("it is empty".to_string());
    }
    for chord in binding.split_whitespace() {
        if let Some(problem) = chord_problem(chord) {
            return Some(problem);
        }
    }
    None
}

fn chord_problem(chord: &str) -> Option<String> {
    // A bare `+` is the plus key, not an empty modifier list.
    if chord == "+" {
        return None;
    }
    let parts: Vec<&str> = chord.split('+').collect();
    let (key, modifiers) = parts.split_last().expect("split always yields one part");
    for modifier in modifiers {
        if modifier.is_empty() {
            return Some(format!("`{chord}` has an empty modifier"));
        }
        if !MODIFIERS.contains(&modifier.to_ascii_lowercase().as_str()) {
            // Listed from `MODIFIERS`: a typed-out list was already wrong (it never mentioned
            // `cmd`).
            return Some(format!(
                "`{modifier}` is not a modifier; use {}",
                MODIFIERS.join(", ")
            ));
        }
    }
    if key.is_empty() {
        return Some(format!("`{chord}` ends with a modifier and no key"));
    }
    if key.chars().count() == 1 || KEY_NAMES.contains(&key.to_ascii_lowercase().as_str()) {
        return None;
    }
    Some(format!("`{key}` is not a key name"))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyBindings {
    overrides: BTreeMap<String, String>,
}

/// `mod` is Postio's platform-neutral modifier: Control on freedesktop, Command on macOS. Resolved
/// once, when bindings are read, so a synced `config.toml` means the same on both and menu and
/// resolver agree (#586). `Delete` becomes BackSpace on Apple, the key a Mac labels delete. `ctrl`
/// stays literal: whoever writes it means Control.
pub fn expand_mod(binding: &str, platform: Platform) -> String {
    let primary = match platform {
        Platform::Apple => "cmd",
        Platform::Freedesktop => "ctrl",
    };
    binding
        .split(' ')
        .map(|chord| {
            chord
                .split('+')
                // Case-insensitive like every other modifier: `chord_problem` lowercases before
                // checking, so `Ctrl+k` already works and `Mod` must too. The key half is
                // untouched: shift is written into the character, so lowercasing the chord would
                // turn `mod+K` into another binding.
                .map(|part| {
                    if part.eq_ignore_ascii_case("mod") {
                        primary
                    } else if part == "Delete" && platform == Platform::Apple {
                        // The key every Mac labels "delete" is BackSpace; forward delete needs fn.
                        "BackSpace"
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl KeyBindings {
    /// Not the key the command answers to: a command with no `[keys]` entry still has its registry
    /// default. Ask `postio_core::config::Keymap`.
    pub fn binding(&self, command: &str) -> Option<&str> {
        self.overrides.get(command).map(String::as_str)
    }

    pub fn overrides(&self) -> &BTreeMap<String, String> {
        &self.overrides
    }

    pub fn overrides_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.overrides
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

/// The settings pane's write path (#881): only the `[keys]` table is touched, so comments elsewhere
/// survive.
pub fn patch_keys(text: &str, overrides: &BTreeMap<String, String>) -> Result<String> {
    // An empty map serializes as a bare dangling header, so the empty case only removes the
    // section.
    if overrides.is_empty() {
        let mut doc = crate::splice::parse(text)?;
        doc.as_table_mut().remove("keys");
        return Ok(doc.to_string());
    }
    crate::splice::replace_section(text, "keys", &KeysOnly { keys: overrides })
}

#[derive(Serialize)]
struct KeysOnly<'a> {
    keys: &'a BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_bindings_are_accepted() {
        for binding in [
            "j",
            "A",
            "/",
            "?",
            "+",
            "Return",
            "escape",
            "ctrl+k",
            "ctrl+shift+plus",
            "g s",
            "alt+Left",
            "super+f1",
        ] {
            assert_eq!(binding_problem(binding), None, "{binding}");
        }
    }

    #[test]
    fn broken_bindings_say_why() {
        for (binding, needle) in [
            ("", "empty"),
            ("   ", "empty"),
            ("ctrl+", "no key"),
            ("hyper+a", "not a modifier"),
            ("Retrun", "not a key name"),
            ("+ctrl+a", "empty modifier"),
            ("j ctrl+", "no key"),
        ] {
            let problem = binding_problem(binding)
                .unwrap_or_else(|| panic!("`{binding}` should be rejected"));
            assert!(problem.contains(needle), "{binding}: {problem}");
        }
    }

    #[test]
    fn unknown_commands_have_no_binding() {
        assert_eq!(KeyBindings::default().binding("teleport"), None);
    }

    #[test]
    fn patch_keys_rewrites_only_the_keys_table_leaving_everything_else_verbatim() {
        let original = "\
# a hand-written comment nobody wants to lose
[ui]
theme = \"dark\" # inline comment, also not to be lost

[keys]
archive = \"a\"
";
        let mut overrides = BTreeMap::new();
        overrides.insert("archive".to_string(), "shift+a".to_string());

        let patched = patch_keys(original, &overrides).expect("patches");

        assert!(
            patched.contains("# a hand-written comment nobody wants to lose"),
            "a comment outside [keys] must survive verbatim: {patched}"
        );
        assert!(
            patched.contains("theme = \"dark\" # inline comment, also not to be lost"),
            "an unrelated section's own formatting must survive verbatim: {patched}"
        );

        let reparsed = crate::Config::from_toml_str(&patched).expect("still parses");
        assert_eq!(reparsed.keys.binding("archive"), Some("shift+a"));
    }

    #[test]
    fn patch_keys_removes_the_table_entirely_once_every_override_is_cleared() {
        let original = "[keys]\narchive = \"shift+a\"\n";
        let patched = patch_keys(original, &BTreeMap::new()).expect("patches");
        assert!(
            !patched.contains("[keys"),
            "no overrides left means no dangling [keys] header: {patched}"
        );
    }

    #[test]
    fn patch_keys_adds_the_table_when_it_did_not_exist_before() {
        let original = "[ui]\ntheme = \"dark\"\n";
        let mut overrides = BTreeMap::new();
        overrides.insert("archive".to_string(), "shift+a".to_string());

        let patched = patch_keys(original, &overrides).expect("patches");
        assert!(patched.contains("[keys]"));
        assert!(patched.contains("theme = \"dark\""));

        let reparsed = crate::Config::from_toml_str(&patched).expect("still parses");
        assert_eq!(reparsed.keys.binding("archive"), Some("shift+a"));
    }
}

#[cfg(test)]
mod mod_token_tests {
    use super::*;
    use crate::paths::Platform;

    #[test]
    fn mod_is_control_on_freedesktop_and_command_on_apple() {
        assert_eq!(
            expand_mod("mod+k", Platform::Freedesktop),
            "ctrl+k".to_string()
        );
        assert_eq!(expand_mod("mod+k", Platform::Apple), "cmd+k".to_string());
    }

    #[test]
    fn delete_is_the_key_a_mac_calls_delete() {
        assert_eq!(expand_mod("Delete", Platform::Apple), "BackSpace");
        assert_eq!(expand_mod("Delete", Platform::Freedesktop), "Delete");
        assert_eq!(expand_mod("mod+Delete", Platform::Apple), "cmd+BackSpace");
        assert_eq!(
            expand_mod("g Delete", Platform::Apple),
            "g BackSpace",
            "chord by chord"
        );
        assert_eq!(
            expand_mod("BackSpace", Platform::Apple),
            "BackSpace",
            "nothing else moves"
        );
    }

    #[test]
    fn the_token_is_recognised_however_it_is_capitalised() {
        for spelling in ["mod+k", "Mod+k", "MOD+k", "mOd+k"] {
            assert_eq!(
                expand_mod(spelling, Platform::Freedesktop),
                "ctrl+k".to_string(),
                "`{spelling}` should expand like any other modifier"
            );
        }
    }

    #[test]
    fn only_the_modifier_is_case_insensitive_never_the_key() {
        assert_eq!(
            expand_mod("Mod+K", Platform::Freedesktop),
            "ctrl+K".to_string()
        );
        assert_eq!(
            expand_mod("MOD+Return", Platform::Apple),
            "cmd+Return".to_string()
        );
    }

    #[test]
    fn a_word_merely_containing_mod_is_not_the_token() {
        assert_eq!(
            expand_mod("ctrl+model", Platform::Freedesktop),
            "ctrl+model".to_string()
        );
    }

    #[test]
    fn ctrl_stays_literal_on_both() {
        assert_eq!(expand_mod("ctrl+b", Platform::Apple), "ctrl+b".to_string());
        assert_eq!(
            expand_mod("ctrl+b", Platform::Freedesktop),
            "ctrl+b".to_string()
        );
    }

    #[test]
    fn mod_composes_with_other_modifiers() {
        assert_eq!(
            expand_mod("mod+shift+n", Platform::Apple),
            "cmd+shift+n".to_string()
        );
        assert_eq!(
            expand_mod("mod+shift+n", Platform::Freedesktop),
            "ctrl+shift+n".to_string()
        );
    }

    #[test]
    fn a_binding_with_no_mod_is_untouched() {
        for binding in ["a", "A", "g g", "shift+Tab", "Return"] {
            assert_eq!(expand_mod(binding, Platform::Apple), binding.to_string());
        }
    }

    #[test]
    fn an_override_is_expanded_too() {
        let written = "mod+shift+a";
        assert_eq!(expand_mod(written, Platform::Apple), "cmd+shift+a");
        assert_eq!(expand_mod(written, Platform::Freedesktop), "ctrl+shift+a");
    }

    #[test]
    fn the_file_keeps_what_the_user_wrote() {
        // Round-tripping must not rewrite `mod+` into the saving platform's key, or syncing between
        // machines would fight itself.
        let mut bindings = KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("archive".to_string(), "mod+shift+a".to_string());
        assert_eq!(bindings.binding("archive"), Some("mod+shift+a"));
    }
}
