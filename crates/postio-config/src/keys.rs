//! `[keys]` — command id to key binding.
//!
//! The file holds *overrides only*: [`DEFAULT_BINDINGS`] is the built-in map
//! and `postio-core`'s command registry takes its default binding from here, so
//! there is exactly one source of truth. Keeping the file override-only means a
//! round trip never rewrites bindings the user did not set.
//!
//! Binding syntax is deliberately untyped at this layer — the keymap resolver
//! in `postio-gtk` parses `"a"`, `"A"`, `"ctrl+k"`, `"g s"` — so a binding for a
//! command this version does not know is preserved rather than dropped.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The built-in bindings, taken from the design canvas.
pub const DEFAULT_BINDINGS: &[(&str, &str)] = &[
    ("next_message", "j"),
    ("prev_message", "k"),
    ("open_message", "Return"),
    ("back", "Escape"),
    ("thread", "t"),
    ("archive", "a"),
    ("archive_thread", "A"),
    ("undo", "u"),
    ("reply", "e"),
    ("reply_all", "E"),
    ("forward", "f"),
    ("compose", "c"),
    ("bold", "ctrl+b"),
    ("italic", "ctrl+i"),
    ("bullet_list", "ctrl+shift+8"),
    ("numbered_list", "ctrl+shift+7"),
    ("insert_link", "ctrl+shift+k"),
    ("quote_block", "ctrl+shift+9"),
    ("search", "/"),
    ("command_palette", "ctrl+k"),
    ("cheat_sheet", "?"),
    ("settings", "ctrl+comma"),
    ("add_account", "ctrl+shift+n"),
    ("edit_config", "ctrl+e"),
];

/// Modifiers a binding may combine with a key.
///
/// GTK spells the last two `Super`/`Meta`; both are accepted so a binding
/// copied out of another application's docs still works.
pub const MODIFIERS: &[&str] = &["ctrl", "control", "alt", "shift", "super", "meta"];

/// Multi-character key names Postio understands, beyond single characters.
///
/// These are GDK key names, which is what the keymap resolver in `postio-gtk`
/// looks up, spelled case-insensitively here because people type `escape`.
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
            return Some(format!(
                "`{modifier}` is not a modifier; use ctrl, alt, shift or super"
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

/// The `[keys]` section: user overrides on top of [`DEFAULT_BINDINGS`].
///
/// ```toml
/// [keys]
/// archive = "a"
/// archive_thread = "A"
/// undo = "u"
/// reply = "e"
/// thread = "t"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyBindings {
    overrides: BTreeMap<String, String>,
}

impl KeyBindings {
    /// The binding for a command: the user's override if present, otherwise the
    /// built-in default, otherwise `None`.
    pub fn binding(&self, command: &str) -> Option<&str> {
        if let Some(custom) = self.overrides.get(command) {
            return Some(custom.as_str());
        }
        DEFAULT_BINDINGS
            .iter()
            .find(|(id, _)| *id == command)
            .map(|(_, key)| *key)
    }

    /// Only what the config file set, in file order-independent key order.
    pub fn overrides(&self) -> &BTreeMap<String, String> {
        &self.overrides
    }

    /// Mutable access, for the settings panel.
    pub fn overrides_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.overrides
    }

    /// Defaults merged with overrides — the map the keymap resolver wants.
    pub fn resolved(&self) -> BTreeMap<String, String> {
        let mut map: BTreeMap<String, String> = DEFAULT_BINDINGS
            .iter()
            .map(|(id, key)| ((*id).to_string(), (*key).to_string()))
            .collect();
        map.extend(
            self.overrides
                .iter()
                .map(|(id, key)| (id.clone(), key.clone())),
        );
        map
    }

    /// Whether the file set anything at all.
    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_binding_ids_are_unique() {
        let mut ids: Vec<&str> = DEFAULT_BINDINGS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate command id in DEFAULT_BINDINGS");
    }

    #[test]
    fn every_default_binding_is_syntactically_valid() {
        for (command, binding) in DEFAULT_BINDINGS {
            assert_eq!(binding_problem(binding), None, "{command} = {binding}");
        }
    }

    #[test]
    fn default_bindings_do_not_collide() {
        let mut seen: Vec<&str> = DEFAULT_BINDINGS.iter().map(|(_, key)| *key).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "two commands share a default binding");
    }

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
}
