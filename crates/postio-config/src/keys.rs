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

use crate::paths::Platform;
use crate::{ConfigError, Result};

/// The built-in bindings, taken from the design canvas.
pub const DEFAULT_BINDINGS: &[(&str, &str)] = &[
    ("next_message", "j"),
    ("prev_message", "k"),
    ("open_message", "Return"),
    ("back", "Escape"),
    ("archive", "a"),
    ("archive_thread", "A"),
    ("undo", "u"),
    ("reply", "e"),
    ("reply_all", "E"),
    ("forward", "f"),
    ("compose", "c"),
    ("bold", "mod+b"),
    ("italic", "mod+i"),
    ("bullet_list", "mod+shift+8"),
    ("numbered_list", "mod+shift+7"),
    ("insert_link", "mod+shift+k"),
    ("quote_block", "mod+shift+9"),
    ("search", "/"),
    ("command_palette", "mod+k"),
    ("cheat_sheet", "?"),
    ("settings", "mod+comma"),
    ("add_account", "mod+shift+n"),
    ("edit_config", "mod+e"),
];

/// Modifiers a binding may combine with a key.
///
/// GTK spells the last two `Super`/`Meta`; both are accepted so a binding
/// copied out of another application's docs still works.
pub const MODIFIERS: &[&str] = &[
    "mod", "ctrl", "control", "cmd", "command", "alt", "shift", "super", "meta",
];

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
                "`{modifier}` is not a modifier; use mod, ctrl, alt, shift or super"
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

/// The primary accelerator, spelled for a platform.
///
/// `mod` is Postio's platform-neutral modifier: Control on freedesktop,
/// Command on macOS. It exists because the command vocabulary is one table
/// across both frontends (#586) while the *accelerator* genuinely differs —
/// every `ctrl+…` default would be wrong on a Mac, where Control is a
/// different key that means something else.
///
/// The two answers this replaces both fail. **Forking the table per platform**
/// gives two vocabularies that drift, and breaks `[keys]` portability: a
/// `config.toml` synced between a desktop and a laptop would mean different
/// things. **Translating at render time** would draw ⌘K while the resolver
/// still matched Control — the menu saying one thing and the keyboard doing
/// another.
///
/// So it is resolved once, when the bindings are read, and everything
/// downstream sees a concrete accelerator.
///
/// `ctrl` stays literal on both. Somebody who writes it means Control, macOS
/// genuinely uses it, and quietly turning their binding into Command would be
/// Postio overriding a stated choice.
pub fn expand_mod(binding: &str, platform: Platform) -> String {
    let primary = match platform {
        Platform::Apple => "cmd",
        Platform::Freedesktop => "ctrl",
    };
    // Chord by chord: a sequence like `g mod+k` has to expand in place.
    binding
        .split(' ')
        .map(|chord| {
            chord
                .split('+')
                // Case-insensitive, like every other modifier in this
                // syntax: `chord_problem` lowercases before checking
                // `MODIFIERS` and the resolver lowercases before matching, so
                // `Ctrl+k` already validates and resolves. Matching `mod`
                // exactly made it the one spelling that passed validation and
                // then failed to work.
                //
                // The *key* half is untouched, and must be: shift is written
                // into the character, so lowercasing the whole chord would
                // turn `mod+K` into a different binding.
                .map(|part| match part.eq_ignore_ascii_case("mod") {
                    true => primary,
                    false => part,
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
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

    /// The binding in force, with `mod` resolved for `platform`.
    ///
    /// What a menu draws and what the resolver matches. [`binding`](Self::binding)
    /// answers what is *written*, which is what the file round-trips; this
    /// answers what it *means* here.
    pub fn binding_on(&self, command: &str, platform: Platform) -> Option<String> {
        self.binding(command)
            .map(|binding| expand_mod(binding, platform))
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

/// Rewrites just `[keys]` in `text` to hold exactly `overrides`, leaving
/// everything else in the file untouched — the settings pane's write path
/// (#881), the same shape [`crate::patch_filters`] already established:
/// `toml_edit`'s format-preserving document model, touching only the one
/// table this pane owns, so a comment anywhere else survives verbatim.
///
/// `KeyBindings` is `#[serde(transparent)]`, so `[keys]`'s own shape is
/// already just `command = "binding"` pairs with no wrapper table the way
/// `[filters.<key>]` needs one — but `toml_edit`'s fragment splice still
/// needs *some* struct to serialize from, hence [`KeysOnly`].
pub fn patch_keys(text: &str, overrides: &BTreeMap<String, String>) -> Result<String> {
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| ConfigError::parse(None, &err))?;
    doc.as_table_mut().remove("keys");

    // As `patch_filters` documents: an empty map still serializes as a bare
    // `[keys]` header, which would reintroduce the dangling header this
    // exists to avoid -- so the empty case skips serialization entirely.
    if !overrides.is_empty() {
        let fragment = toml::to_string(&KeysOnly { keys: overrides })
            .map_err(|err| ConfigError::Serialize(err.to_string()))?;
        let fragment_doc = fragment
            .parse::<toml_edit::DocumentMut>()
            .map_err(|err| ConfigError::parse(None, &err))?;
        if let Some(item) = fragment_doc.as_table().get("keys") {
            doc.as_table_mut().insert("keys", item.clone());
        }
    }
    Ok(doc.to_string())
}

/// Serializes as just `[keys]`, with no other section — [`patch_keys`]'s
/// bridge from `toml`'s serde-derived output to a fragment `toml_edit` can
/// splice in, the same role `filters.rs`'s own `FiltersOnly` plays for
/// `[filters]`.
#[derive(Serialize)]
struct KeysOnly<'a> {
    keys: &'a BTreeMap<String, String>,
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

    // -- Acceptance: the settings panel patches [keys] only (#881) ---------

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
        // The whole point. `mod` is "the primary accelerator", which is a
        // different physical key on each platform — and Control on macOS is
        // not it, it means something else there.
        assert_eq!(
            expand_mod("mod+k", Platform::Freedesktop),
            "ctrl+k".to_string()
        );
        assert_eq!(expand_mod("mod+k", Platform::Apple), "cmd+k".to_string());
    }

    #[test]
    fn the_token_is_recognised_however_it_is_capitalised() {
        // Every other modifier in this syntax already is. `chord_problem`
        // lowercases before checking `MODIFIERS`, and the resolver lowercases
        // before matching, so `Ctrl+k` validates *and* resolves. Leaving `mod`
        // exact made it the one spelling that passes validation and then fails
        // to work -- and a user has no way to predict that `Ctrl` is fine and
        // `Mod` is not.
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
        // Shift is written into the character (`A` is what holding shift
        // gives), so the key half is case-*sensitive* by design. Lowercasing
        // the whole chord to fix the modifier would silently turn `mod+K` into
        // a different binding.
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
        // Matching on the whole `+`-separated part rather than a substring.
        // `KEY_NAMES` has no `model` in it today, but a key or a future
        // modifier that starts with these three letters must not be rewritten.
        assert_eq!(
            expand_mod("ctrl+model", Platform::Freedesktop),
            "ctrl+model".to_string()
        );
    }

    #[test]
    fn ctrl_stays_literal_on_both() {
        // A user who writes `ctrl` means Control. macOS genuinely uses it, and
        // silently turning their binding into Command would be Postio
        // overriding a stated choice.
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
    fn every_default_resolves_to_what_linux_has_today() {
        // The argument for doing this now rather than when macOS ships
        // bindings: on freedesktop the new table resolves to exactly the old
        // one, so nothing a Linux user sees changes.
        let expected: &[(&str, &str)] = &[
            ("command_palette", "ctrl+k"),
            ("settings", "ctrl+comma"),
            ("add_account", "ctrl+shift+n"),
            ("bold", "ctrl+b"),
            ("insert_link", "ctrl+shift+k"),
        ];
        for (command, want) in expected {
            let bindings = KeyBindings::default();
            assert_eq!(
                bindings
                    .binding_on(command, Platform::Freedesktop)
                    .as_deref(),
                Some(*want),
                "{command} changed for Linux users"
            );
        }
    }

    #[test]
    fn the_same_defaults_are_command_on_apple() {
        let bindings = KeyBindings::default();
        assert_eq!(
            bindings
                .binding_on("command_palette", Platform::Apple)
                .as_deref(),
            Some("cmd+k")
        );
    }

    #[test]
    fn an_override_is_expanded_too() {
        // A `config.toml` synced between a Linux desktop and a Mac has to mean
        // the same thing on both, which is why `mod` is a *config* token
        // rather than a rendering trick.
        let mut bindings = KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("archive".to_string(), "mod+shift+a".to_string());

        assert_eq!(
            bindings.binding_on("archive", Platform::Apple).as_deref(),
            Some("cmd+shift+a")
        );
        assert_eq!(
            bindings
                .binding_on("archive", Platform::Freedesktop)
                .as_deref(),
            Some("ctrl+shift+a")
        );
    }

    #[test]
    fn the_file_keeps_what_the_user_wrote() {
        // Round-tripping must not rewrite `mod+` into whichever platform saved
        // it, or syncing the file between machines would fight itself.
        let mut bindings = KeyBindings::default();
        bindings
            .overrides_mut()
            .insert("archive".to_string(), "mod+shift+a".to_string());
        assert_eq!(bindings.binding("archive"), Some("mod+shift+a"));
    }
}

#[cfg(test)]
mod mod_token_validity_tests {
    use super::*;
    use crate::paths::Platform;

    #[test]
    fn expansion_always_produces_a_binding_the_validator_accepts() {
        // `cmd` only ever appears as expansion output, so nothing else would
        // catch it being absent from MODIFIERS -- and the failure would be a
        // Mac where every default is rejected as invalid.
        for platform in [Platform::Freedesktop, Platform::Apple] {
            for (command, binding) in DEFAULT_BINDINGS {
                let expanded = expand_mod(binding, platform);
                assert_eq!(
                    binding_problem(&expanded),
                    None,
                    "{command} expands to `{expanded}`, which the validator rejects"
                );
            }
        }
    }

    #[test]
    fn mod_is_spellable_in_a_config_file() {
        assert_eq!(binding_problem("mod+shift+a"), None);
    }
}
