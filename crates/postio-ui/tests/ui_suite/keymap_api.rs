use postio_config::paths::Platform;
use postio_ui::keymap::{Chord, Key, KeyContext, Keymap, Modifiers, Resolver};

fn chord(text: &str) -> Chord {
    text.parse().expect("a chord")
}

#[test]
fn a_platform_key_becomes_the_chord_the_binding_was_written_as() {
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
    // GDK reports Return as `\r` and the name "Return"; the character must not win.
    assert_eq!(
        Chord::from_platform_key(Some('\r'), Some("Return"), Modifiers::NONE),
        Some(chord("Return"))
    );
}

#[test]
fn a_key_this_build_has_no_name_for_is_no_chord() {
    assert_eq!(
        Chord::from_platform_key(None, Some("dead_acute"), Modifiers::NONE),
        None
    );
    assert_eq!(Chord::from_platform_key(None, None, Modifiers::NONE), None);
}

#[test]
fn the_primary_modifier_parses_the_way_this_platform_spells_it() {
    // `mod` must parse here: a modifier this resolver does not know makes every `mod+…` default
    // unparseable on Apple.
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
    // Defaults live in the registry and `expand_mod` is applied per platform; nothing else asserts
    // a default's token is one this resolver can read (ADR 0019 Q7).
    for platform in [Platform::Freedesktop, Platform::Apple] {
        let keymap = postio_core::Keymap::resolve_on(&Default::default(), platform);
        let (_, problems) = Resolver::from_commands(&keymap);
        assert!(
            problems.is_empty(),
            "{platform:?} could not resolve its own defaults: {problems:?}"
        );
    }
}

#[test]
fn a_picker_digest_and_filtered_view_fall_back_to_global_only() {
    use postio_core::{Context, Keymap as Commands};
    use postio_ui::keymap::Outcome;

    let (mut resolver, problems) = Resolver::from_commands(Commands::defaults());
    assert!(
        problems.is_empty(),
        "the defaults do not resolve: {problems:?}"
    );
    let now = std::time::Instant::now();
    let mut press = |context: Context, key: &str| {
        resolver.press(&chord(key), KeyContext::from(context), false, now)
    };

    assert_eq!(
        press(Context::List, "x"),
        Outcome::Command("toggle_selection".to_owned()),
        "the control: `x` selects a row in the list"
    );
    for context in [Context::Picker, Context::Digest, Context::Filtered] {
        assert_eq!(
            press(context, "Escape"),
            Outcome::Command("back".to_owned()),
            "Escape does not leave {context}"
        );
    }
    assert_eq!(
        press(Context::Picker, "x"),
        Outcome::Unhandled,
        "`x` in a picker reached through to the list underneath it"
    );
}

/// A punctuation key has two spellings (the character and the keysym name); both must be one chord.
#[test]
fn an_exclamation_mark_parses_by_either_name_and_resolves() {
    use postio_ui::keymap::Outcome;

    assert_eq!(
        "exclam".parse::<Chord>(),
        Ok(chord("!")),
        "the keysym name is not the key"
    );
    assert_eq!(
        Key::Char('!').keysym_name(),
        "exclam",
        "a menu cannot draw `!` from a name GTK has no keyval for"
    );
    assert_eq!(
        postio_config::keys::binding_problem("exclam"),
        None,
        "the settings validator refuses a spelling the resolver accepts"
    );
    for character in [Some('!'), None] {
        assert_eq!(
            Chord::from_platform_key(character, Some("exclam"), Modifiers::SHIFT),
            Some(chord("!")),
            "{character:?}"
        );
    }

    let mut keymap = Keymap::new();
    keymap
        .bind(KeyContext::List, "!", "toggle_has_action")
        .unwrap();
    let mut resolver = Resolver::new(keymap);
    let pressed =
        Chord::from_platform_key(Some('!'), Some("exclam"), Modifiers::SHIFT).expect("a chord");
    assert_eq!(
        resolver.press(&pressed, KeyContext::List, false, std::time::Instant::now()),
        Outcome::Command("toggle_has_action".to_owned()),
        "pressing `!` did not reach the command bound to it"
    );
}

/// A key the one keymap keeps for another app is bound to nothing here: `alt+e` is the terminal
/// composer's editor key.
#[test]
fn a_key_another_app_keeps_is_bound_to_nothing_here() {
    use postio_core::{Context, Frontend, Keymap as Commands};
    use postio_ui::keymap::Outcome;

    let now = std::time::Instant::now();
    let press = |frontend: Frontend, context: Context, key: &str| {
        let commands = Commands::resolve_on(&Default::default(), Platform::Freedesktop);
        let (mut resolver, problems) = Resolver::from_commands_for(&commands, frontend);
        assert!(problems.is_empty(), "{frontend:?}: {problems:?}");
        resolver.press(&chord(key), KeyContext::from(context), false, now)
    };

    assert_eq!(
        press(Frontend::Focus, Context::Composer, "alt+shift+e"),
        Outcome::Unhandled,
        "Focus answered the terminal composer's `alt+shift+e`"
    );
    assert_eq!(
        press(Frontend::Terminal, Context::Composer, "alt+shift+e"),
        Outcome::Command("edit_externally".to_owned()),
        "the terminal lost its editor key"
    );
    assert_eq!(
        press(Frontend::Focus, Context::Composer, "alt+e"),
        Outcome::Command("reply_to_latest".to_owned())
    );
    assert_eq!(
        press(Frontend::Terminal, Context::Composer, "alt+e"),
        Outcome::Unhandled
    );
    for app in [Frontend::Terminal, Frontend::Focus] {
        assert_eq!(
            press(app, Context::List, "e"),
            Outcome::Command("archive".to_owned()),
            "{app:?} lost a key every app has"
        );
        assert_eq!(
            press(app, Context::List, "y"),
            Outcome::Command("accept_invite".to_owned()),
            "{app:?} lost Focus's `y`"
        );
    }
}
