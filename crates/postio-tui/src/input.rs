//! Keys from a terminal to the one keymap: a crossterm event is reduced to character, name and
//! modifiers and resolved by `postio_ui::keymap`, with the same `[keys]` overrides as the desktop.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use postio_ui::keymap::{Chord, KeyContext, Modifiers, Outcome, Resolver};

/// `None` for a release, or a key with no name here.
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

#[derive(Debug)]
pub struct Keys {
    resolver: Resolver,
    commands: postio_core::Keymap,
}

impl Keys {
    pub fn new(commands: &postio_core::Keymap) -> (Keys, Vec<String>) {
        // A key the one keymap keeps for another app is bound to nothing here.
        let (resolver, problems) =
            Resolver::from_commands_for(commands, postio_core::Frontend::Terminal);
        (
            Keys {
                resolver,
                commands: commands.clone(),
            },
            problems,
        )
    }

    pub fn key_for(&self, context: KeyContext, command: &str) -> Option<String> {
        context.chain().iter().find_map(|context| {
            self.resolver
                .keymap()
                .binding_for(*context, command)
                .map(ToString::to_string)
        })
    }

    pub fn keymap(&self) -> &postio_core::Keymap {
        &self.commands
    }

    /// Whether `key` is `command`'s first key, asked without touching a half-typed sequence.
    /// Quitting
    /// is recognised this way before any surface sees the key; an alternate (`ctrl+w`) is left to
    /// the surface, where a text field may use it.
    pub fn is_primary(&self, key: &KeyEvent, command: postio_core::CommandId) -> bool {
        let Some(chord) = chord_of(key) else {
            return false;
        };
        self.commands
            .bindings(command)
            .first()
            .and_then(|binding| binding.parse::<postio_ui::keymap::Binding>().ok())
            .is_some_and(|binding| binding.chords() == [chord])
    }

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
        Keys::new(&postio_core::Keymap::for_terminal(&bindings)).0
    }

    #[test]
    fn a_letter_is_the_registrys_command() {
        let mut keys = keys(&[]);
        assert_eq!(
            keys.press(
                &key(KeyCode::Char('e'), KeyModifiers::NONE),
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
        // A terminal without the kitty protocol sends alt+Return for the desktop's ctrl+Return.
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
