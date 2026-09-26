//! Keys, from a terminal to the one keymap.
//!
//! The terminal has no keymap of its own. A crossterm key event is reduced
//! to the three things every frontend's is -- the character, the key's name,
//! the modifiers -- and handed to `postio_ui::keymap`, the same resolver the
//! desktop app uses, with the same `[keys]` overrides (Principle II,
//! research R4). A command bound in the registry is bound here, and a
//! rebinding in `config.toml` rebinds it here, without this file knowing.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use postio_ui::keymap::{Chord, KeyContext, Modifiers, Outcome, Resolver};

/// `key` as the keymap spells it; `None` for a release, or a key with no
/// name here.
pub fn chord_of(key: &KeyEvent) -> Option<Chord> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let mut modifiers = Modifiers::NONE;
    for (terminal, keymap) in [
        (KeyModifiers::CONTROL, Modifiers::CTRL),
        (KeyModifiers::ALT, Modifiers::ALT),
        (KeyModifiers::SHIFT, Modifiers::SHIFT),
        (KeyModifiers::SUPER, Modifiers::SUPER),
    ] {
        if key.modifiers.contains(terminal) {
            modifiers = modifiers.with(keymap);
        }
    }
    let function;
    let (character, name) = match key.code {
        KeyCode::Char(character) => (Some(character), None),
        KeyCode::Enter => (None, Some("Return")),
        KeyCode::Esc => (None, Some("Escape")),
        KeyCode::Tab => (None, Some("Tab")),
        // A terminal reports shift+Tab as its own key.
        KeyCode::BackTab => {
            modifiers = modifiers.with(Modifiers::SHIFT);
            (None, Some("Tab"))
        }
        KeyCode::Backspace => (None, Some("BackSpace")),
        KeyCode::Delete => (None, Some("Delete")),
        KeyCode::Insert => (None, Some("Insert")),
        KeyCode::Home => (None, Some("Home")),
        KeyCode::End => (None, Some("End")),
        KeyCode::PageUp => (None, Some("Page_Up")),
        KeyCode::PageDown => (None, Some("Page_Down")),
        KeyCode::Up => (None, Some("Up")),
        KeyCode::Down => (None, Some("Down")),
        KeyCode::Left => (None, Some("Left")),
        KeyCode::Right => (None, Some("Right")),
        KeyCode::Menu => (None, Some("Menu")),
        KeyCode::F(number) => {
            function = format!("F{number}");
            (None, Some(function.as_str()))
        }
        _ => return None,
    };
    Chord::from_platform_key(character, name, modifiers)
}

/// The keymap, resolving terminal keys.
#[derive(Debug)]
pub struct Keys {
    resolver: Resolver,
    /// The bindings the resolver was built from, for the palette and the
    /// cheat sheet, which list them.
    commands: postio_core::Keymap,
}

impl Keys {
    /// The registry's bindings with `[keys]` applied, and whatever could not
    /// be honoured.
    pub fn new(commands: &postio_core::Keymap) -> (Keys, Vec<String>) {
        let (resolver, problems) = Resolver::from_commands(commands);
        (
            Keys {
                resolver,
                commands: commands.clone(),
            },
            problems,
        )
    }

    /// The key that runs `command` with the keyboard in `context`, as the
    /// cheat sheet spells it -- whatever `[keys]` bound it to.
    pub fn key_for(&self, context: KeyContext, command: &str) -> Option<String> {
        context.chain().iter().find_map(|context| {
            self.resolver
                .keymap()
                .binding_for(*context, command)
                .map(ToString::to_string)
        })
    }

    /// The keymap in force: the registry's bindings with `[keys]` applied.
    pub fn keymap(&self) -> &postio_core::Keymap {
        &self.commands
    }

    /// What `key` means with the keyboard in `context`.
    pub fn press(&mut self, key: &KeyEvent, context: KeyContext, in_text_entry: bool) -> Outcome {
        match chord_of(key) {
            Some(chord) => self
                .resolver
                .press(&chord, context, in_text_entry, Instant::now()),
            None => Outcome::Unhandled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn keys(overrides: &[(&str, &str)]) -> Keys {
        let mut bindings = postio_config::KeyBindings::default();
        for (command, binding) in overrides {
            bindings
                .overrides_mut()
                .insert((*command).to_owned(), (*binding).to_owned());
        }
        Keys::new(&postio_core::Keymap::resolve(&bindings)).0
    }

    #[test]
    fn a_letter_is_the_registrys_command() {
        let mut keys = keys(&[]);
        assert_eq!(
            keys.press(
                &key(KeyCode::Char('a'), KeyModifiers::NONE),
                KeyContext::List,
                false
            ),
            Outcome::Command("archive".into())
        );
    }

    #[test]
    fn a_rebinding_in_config_is_honoured_here_too() {
        let mut keys = keys(&[("archive", "z")]);
        assert_eq!(
            keys.press(
                &key(KeyCode::Char('z'), KeyModifiers::NONE),
                KeyContext::List,
                false
            ),
            Outcome::Command("archive".into())
        );
    }

    #[test]
    fn a_legacy_alternate_reaches_its_command() {
        // T017: a terminal without the kitty protocol sends alt+Return for
        // what the desktop binds to ctrl+Return.
        let mut keys = keys(&[]);
        assert_eq!(
            keys.press(
                &key(KeyCode::Enter, KeyModifiers::ALT),
                KeyContext::Composer,
                true
            ),
            Outcome::Command("send".into())
        );
    }

    #[test]
    fn named_keys_and_modifiers_are_spelled_as_the_keymap_spells_them() {
        let chord = |code, modifiers| chord_of(&key(code, modifiers)).map(|c| c.to_string());
        assert_eq!(
            chord(KeyCode::Enter, KeyModifiers::CONTROL),
            Some("ctrl+Return".into())
        );
        assert_eq!(
            chord(KeyCode::BackTab, KeyModifiers::SHIFT),
            Some("shift+Tab".into())
        );
        assert_eq!(
            chord(KeyCode::Char('A'), KeyModifiers::SHIFT),
            Some("A".into())
        );
        assert_eq!(
            chord(KeyCode::Char('k'), KeyModifiers::CONTROL),
            Some("ctrl+k".into())
        );
        assert_eq!(chord(KeyCode::F(5), KeyModifiers::NONE), Some("F5".into()));
        assert_eq!(
            chord(KeyCode::Char(' '), KeyModifiers::NONE),
            Some("Space".into())
        );
        assert_eq!(
            chord(KeyCode::PageDown, KeyModifiers::NONE),
            Some("Page_Down".into())
        );
    }

    #[test]
    fn a_key_release_is_no_press() {
        let mut release = key(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(chord_of(&release), None);
    }
}
