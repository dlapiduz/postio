//! The platform-neutral half of what was the GDK bridge (#568, ADR 0019 Q4).
//!
//! `from_platform_key` is asserted against the same table the GDK path uses,
//! with no toolkit and no display; `trigger_for_command` is what a frontend
//! renders a native accelerator from.

use postio_config::paths::Platform;
use postio_ui::keymap::{Chord, Key, KeyContext, Keymap, Modifiers, Resolver, trigger_for_command};

fn chord(text: &str) -> Chord {
    text.parse().expect("a chord")
}

#[test]
fn a_platform_key_becomes_the_chord_the_binding_was_written_as() {
    // The same table `from_key_event` is asserted against on the GTK side:
    // (character it would type, key name when it types nothing, modifiers).
    for (character, name, modifiers, expected) in [
        (Some('a'), Some("a"), Modifiers::NONE, "a"),
        (Some('A'), Some("A"), Modifiers::SHIFT, "A"),
        (Some('k'), Some("k"), Modifiers::CTRL, "ctrl+k"),
        (None, Some("Return"), Modifiers::NONE, "Return"),
        (None, Some("Return"), Modifiers::CTRL, "ctrl+Return"),
        (None, Some("Escape"), Modifiers::NONE, "Escape"),
        (None, Some("Tab"), Modifiers::SHIFT, "shift+Tab"),
        (Some('?'), Some("question"), Modifiers::SHIFT, "?"),
        (Some('/'), Some("slash"), Modifiers::NONE, "/"),
        (None, Some("Page_Up"), Modifiers::NONE, "Page_Up"),
        (None, Some("F5"), Modifiers::NONE, "F5"),
        (Some(' '), Some("space"), Modifiers::NONE, "Space"),
    ] {
        let built = Chord::from_platform_key(character, name, modifiers)
            .unwrap_or_else(|| panic!("{name:?} produced no chord"));
        assert_eq!(built, chord(expected), "{character:?}/{name:?}");
    }
}

#[test]
fn a_control_character_falls_back_to_the_key_name() {
    // GDK reports Return as the control character `\r` *and* the name
    // "Return"; the character must not win.
    assert_eq!(
        Chord::from_platform_key(Some('\r'), Some("Return"), Modifiers::NONE),
        Some(chord("Return"))
    );
}

#[test]
fn a_key_this_build_has_no_name_for_is_no_chord() {
    // A dead key: no character, no recognizable name.
    assert_eq!(
        Chord::from_platform_key(None, Some("dead_acute"), Modifiers::NONE),
        None
    );
    assert_eq!(Chord::from_platform_key(None, None, Modifiers::NONE), None);
}

#[test]
fn the_trigger_for_a_command_is_the_chord_its_binding_starts_and_ends_with() {
    let mut keymap = Keymap::new();
    keymap.bind(KeyContext::List, "a", "archive").unwrap();
    keymap
        .bind(KeyContext::Global, "ctrl+k", "command_palette")
        .unwrap();

    assert_eq!(
        trigger_for_command(&keymap, "archive"),
        Some(Chord::new(Key::Char('a'), Modifiers::NONE))
    );
    assert_eq!(
        trigger_for_command(&keymap, "command_palette"),
        Some(Chord::new(Key::Char('k'), Modifiers::CTRL))
    );
    assert_eq!(trigger_for_command(&keymap, "teleport"), None);
}

#[test]
fn a_sequence_is_not_a_trigger() {
    // `g g` cannot be drawn as a native accelerator; a command bound only to
    // a sequence has no trigger rather than a misleading first half.
    let mut keymap = Keymap::new();
    keymap.bind(KeyContext::List, "g g", "go_to_top").unwrap();

    assert_eq!(trigger_for_command(&keymap, "go_to_top"), None);
}

#[test]
fn a_global_binding_wins_over_a_context_one_as_the_trigger() {
    // A native menu is not in any one context; when a command is bound both
    // globally and per-context, the global chord is the honest accelerator.
    let mut keymap = Keymap::new();
    keymap.bind(KeyContext::List, "x", "do_it").unwrap();
    keymap.bind(KeyContext::Global, "ctrl+x", "do_it").unwrap();

    assert_eq!(
        trigger_for_command(&keymap, "do_it"),
        Some(Chord::new(Key::Char('x'), Modifiers::CTRL))
    );
}

#[test]
fn the_trigger_follows_what_keys_binds() {
    // The [keys] override wins before the trigger is ever computed — the
    // accelerator a menu draws is the key that actually works.
    let mut overrides = postio_config::KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_string(), "ctrl+shift+y".to_string());
    let (keymap, _) = Keymap::from_commands(&postio_core::Keymap::resolve(&overrides));

    assert_eq!(
        trigger_for_command(&keymap, "archive"),
        Some(Chord::new(Key::Char('Y'), Modifiers::CTRL))
    );
}

#[test]
fn the_primary_modifier_parses_the_way_this_platform_spells_it() {
    // `mod` is the primary accelerator and `postio_config::keys::expand_mod`
    // resolves it when the keymap is built: `ctrl` on freedesktop, **`cmd`**
    // on Apple (#669). `postio_config::keys::MODIFIERS` accepts both spellings
    // and `chord_problem` validates them, so `cmd+k` passes every check
    // upstream of here.
    //
    // It has to parse *here* too. This resolver is the only thing that decides
    // whether a chord matches, and a modifier it does not know makes the whole
    // binding unparseable -- which on Apple is every `mod+…` default at once,
    // reported as a keymap problem rather than as a key that does nothing, a
    // long way from where anyone would look.
    for spelling in ["cmd+k", "command+k"] {
        assert_eq!(
            spelling.parse::<Chord>(),
            Ok(Chord::new(Key::Char('k'), Modifiers::SUPER)),
            "{spelling} is a binding this build can be asked to resolve"
        );
    }
}

#[test]
fn every_default_binding_resolves_on_both_platforms() {
    // The class the test above is one member of, and the reason it is not
    // enough on its own: the defaults live in the registry, `expand_mod` is
    // applied to them per platform, and nothing else asserts that the two
    // agree. A default spelled with a token this resolver cannot read is a
    // command with no key on that platform, and the Linux gate cannot see it
    // -- which is exactly the shape ADR 0019 Q7 says to guard against.
    for platform in [Platform::Freedesktop, Platform::Apple] {
        let keymap = postio_core::Keymap::resolve_on(&Default::default(), platform);
        let (_, problems) = Resolver::from_commands(&keymap);
        assert!(
            problems.is_empty(),
            "{platform:?} could not resolve its own defaults: {problems:?}"
        );
    }
}
