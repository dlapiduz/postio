//! GTK's half of the keymap: the GDK bridge and the accelerator renderer.
//!
//! The resolver itself — contexts, sequences, the text-entry rule — lives in
//! [`postio_ui::keymap`] (ADR 0019), because none of it is toolkit-shaped.
//! What is toolkit-shaped is exactly two things, and they are all this module
//! holds: turning a GDK key event into a [`Chord`], and turning the chord
//! bound to a command back into an accelerator string GTK menus can draw.

pub use postio_ui::keymap::*;

use gtk::gdk;

/// The GDK spelling of [`Chord::from_platform_key`].
///
/// An extension trait rather than a method so the chord type itself stays
/// toolkit-free; with this in scope, `Chord::from_key_event(keyval, state)`
/// reads exactly as it did when the chord lived in this crate.
pub trait ChordFromGdk: Sized {
    /// Builds a chord from a GTK key-pressed event.
    ///
    /// Returns `None` for a key this build has no name for — a dead key, a
    /// keypad key with no binding — which the caller should let propagate.
    fn from_key_event(keyval: gdk::Key, state: gdk::ModifierType) -> Option<Self>;
}

impl ChordFromGdk for Chord {
    fn from_key_event(keyval: gdk::Key, state: gdk::ModifierType) -> Option<Self> {
        let mut modifiers = Modifiers::NONE;
        for (mask, modifier) in [
            (gdk::ModifierType::CONTROL_MASK, Modifiers::CTRL),
            (gdk::ModifierType::ALT_MASK, Modifiers::ALT),
            (gdk::ModifierType::SHIFT_MASK, Modifiers::SHIFT),
            (gdk::ModifierType::SUPER_MASK, Modifiers::SUPER),
        ] {
            if state.contains(mask) {
                modifiers = modifiers.with(modifier);
            }
        }
        let name = keyval.name();
        Chord::from_platform_key(keyval.to_unicode(), name.as_deref(), modifiers)
    }
}

/// The chord as a GTK accelerator string — `<Control>k` — or `None` for a
/// chord GTK has no keyval for.
///
/// The inverse of the [`Chord::new`] normalization: an uppercase character
/// unfolds back into `<Shift>` plus its lowercase keyval, which is the key
/// the user actually presses and the label GTK draws for it.
pub fn gtk_accelerator(chord: &Chord) -> Option<String> {
    let (key_name, unfolded_shift) = match &chord.key {
        Key::Char(character) if character.is_uppercase() => {
            let lower = character.to_lowercase().next()?;
            (Key::Char(lower).keysym_name(), true)
        }
        key => (key.keysym_name(), false),
    };
    // A key GTK has no keysym for draws nothing rather than a string the
    // menu cannot parse.
    gdk::Key::from_name(key_name.as_str())?;

    let mut accelerator = String::new();
    if chord.modifiers.contains(Modifiers::CTRL) {
        accelerator.push_str("<Control>");
    }
    if chord.modifiers.contains(Modifiers::ALT) {
        accelerator.push_str("<Alt>");
    }
    if unfolded_shift || chord.modifiers.contains(Modifiers::SHIFT) {
        accelerator.push_str("<Shift>");
    }
    if chord.modifiers.contains(Modifiers::SUPER) {
        accelerator.push_str("<Super>");
    }
    accelerator.push_str(&key_name);
    Some(accelerator)
}

/// The accelerator a menu item for `command` should draw, from the same
/// resolved table every other surface reads.
pub fn accelerator_for_command(keymap: &Keymap, command: &str) -> Option<String> {
    gtk_accelerator(&trigger_for_command(keymap, command)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        text.parse().expect("a chord")
    }

    // -----------------------------------------------------------------------
    // The GDK bridge
    // -----------------------------------------------------------------------

    #[test]
    fn a_key_event_becomes_the_chord_the_binding_was_written_as() {
        for (name, state, expected) in [
            ("a", gdk::ModifierType::empty(), "a"),
            ("A", gdk::ModifierType::SHIFT_MASK, "A"),
            ("k", gdk::ModifierType::CONTROL_MASK, "ctrl+k"),
            ("Return", gdk::ModifierType::empty(), "Return"),
            ("Return", gdk::ModifierType::CONTROL_MASK, "ctrl+Return"),
            ("Escape", gdk::ModifierType::empty(), "Escape"),
            ("Tab", gdk::ModifierType::SHIFT_MASK, "shift+Tab"),
            ("question", gdk::ModifierType::SHIFT_MASK, "?"),
            ("slash", gdk::ModifierType::empty(), "/"),
            ("Page_Up", gdk::ModifierType::empty(), "Page_Up"),
            ("F5", gdk::ModifierType::empty(), "F5"),
            ("space", gdk::ModifierType::empty(), "Space"),
        ] {
            let built = Chord::from_key_event(gdk::Key::from_name(name).unwrap(), state)
                .unwrap_or_else(|| panic!("{name} produced no chord"));
            assert_eq!(built, chord(expected), "{name} with {state:?}");
        }
    }

    #[test]
    fn a_key_event_resolves_the_command_the_canvas_binds_to_it() {
        use std::time::Instant;

        let mut keymap = Keymap::new();
        keymap
            .bind(KeyContext::List, "A", "archive_thread")
            .unwrap();
        let mut resolver = Resolver::new(keymap);

        let pressed = Chord::from_key_event(
            gdk::Key::from_name("A").unwrap(),
            gdk::ModifierType::SHIFT_MASK,
        )
        .expect("a chord");

        assert_eq!(
            resolver.press(&pressed, KeyContext::List, false, Instant::now()),
            Outcome::Command("archive_thread".to_owned()),
            "shift+a is the canvas's A"
        );
    }

    // -----------------------------------------------------------------------
    // The accelerator renderer
    // -----------------------------------------------------------------------

    #[test]
    fn a_chord_renders_as_the_accelerator_the_menu_will_parse() {
        // `keymap_live.rs` proves GTK itself parses these strings back to
        // the same key; this asserts the strings, with no display.
        for (text, expected) in [
            ("ctrl+k", "<Control>k"),
            ("a", "a"),
            // The uppercase chord unfolds back into shift plus the key the
            // user actually presses.
            ("A", "<Shift>a"),
            ("Return", "Return"),
            ("ctrl+Return", "<Control>Return"),
            ("shift+Tab", "<Shift>Tab"),
            ("?", "question"),
            ("Space", "space"),
            ("ctrl+shift+y", "<Control><Shift>y"),
            ("super+F5", "<Super>F5"),
        ] {
            assert_eq!(
                gtk_accelerator(&chord(text)).as_deref(),
                Some(expected),
                "`{text}`"
            );
        }
    }

    #[test]
    fn a_menu_item_draws_the_key_its_command_is_bound_to() {
        let mut keymap = Keymap::new();
        keymap.bind(KeyContext::List, "a", "archive").unwrap();
        keymap.bind(KeyContext::List, "g g", "go_to_top").unwrap();

        assert_eq!(
            accelerator_for_command(&keymap, "archive").as_deref(),
            Some("a")
        );
        assert_eq!(
            accelerator_for_command(&keymap, "go_to_top"),
            None,
            "a sequence draws no accelerator rather than half of one"
        );
    }
}
