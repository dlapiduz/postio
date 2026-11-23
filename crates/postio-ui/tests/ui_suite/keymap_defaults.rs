use postio_config::KeyBindings;
use postio_config::keys::{binding_problem, expand_mod};
use postio_config::paths::Platform;
use postio_core::{CommandId, Context};
use postio_ui::keymap::{Binding, KeyContext, Keymap, Outcome, Resolver};

fn defaults() -> (Keymap, Vec<String>) {
    Keymap::from_commands(&postio_core::Keymap::resolve_on(
        &KeyBindings::default(),
        Platform::Freedesktop,
    ))
}

fn resolver() -> Resolver {
    let (keymap, problems) = defaults();
    assert!(problems.is_empty(), "{problems:?}");
    Resolver::new(keymap)
}

fn press(resolver: &mut Resolver, keys: &str, context: KeyContext) -> Outcome {
    let binding: Binding = keys
        .parse()
        .unwrap_or_else(|error| panic!("{keys}: {error}"));
    let now = std::time::Instant::now();
    let mut outcome = Outcome::Unhandled;
    for chord in binding.chords() {
        outcome = resolver.press(chord, context, false, now);
    }
    outcome
}

#[test]
fn the_whole_default_set_binds_with_nothing_left_unparsed() {
    let (_, problems) = defaults();

    assert!(
        problems.is_empty(),
        "a shipped default that the resolver cannot read costs its command a key: {problems:?}"
    );
}

#[test]
fn no_default_binding_is_shadowed_by_a_shorter_one() {
    let (keymap, _) = defaults();
    let shadowed: Vec<String> = keymap
        .shadowed()
        .iter()
        .map(|(context, binding, command)| format!("{command} = {binding} in {context:?}"))
        .collect();

    assert!(
        shadowed.is_empty(),
        "these bindings can never fire: {shadowed:?}"
    );
}

#[test]
fn the_two_binding_parsers_agree() {
    // Two parsers of one syntax drift unless something holds them together.
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
        assert!(
            binding_problem(binding).is_none() && binding.parse::<Binding>().is_ok(),
            "{binding} should be accepted by both"
        );
    }
    for binding in ["", "   ", "ctrl+", "hyper+a", "Retrun", "j ctrl+"] {
        assert!(
            binding_problem(binding).is_some() && binding.parse::<Binding>().is_err(),
            "{binding} should be rejected by both"
        );
    }

    // The one place the parsers disagree on purpose: `mod` is valid in config and resolved to
    // `ctrl` or `cmd` before the resolver. "Fixing" either side breaks the other platform.
    assert!(
        binding_problem("mod+k").is_none(),
        "`mod` is spellable in config.toml"
    );
    assert!(
        "mod+k".parse::<Binding>().is_err(),
        "`mod` must never reach the resolver unexpanded"
    );
}

#[test]
fn every_binding_the_registry_ships_parses() {
    for spec in postio_core::registry::all() {
        for binding in spec.bindings() {
            let binding = expand_mod(binding, Platform::Freedesktop);
            binding
                .parse::<Binding>()
                .unwrap_or_else(|error| panic!("{} = {binding}: {error}", spec.id));
        }
    }
}

#[test]
fn the_canvas_navigation_set_works_in_the_list() {
    let mut resolver = resolver();

    for (keys, expected) in [
        ("j", CommandId::NextMessage),
        ("k", CommandId::PrevMessage),
        ("u", CommandId::PrevView),
        ("Right", CommandId::ExpandThread),
        ("Left", CommandId::CollapseThread),
        ("Return", CommandId::OpenMessage),
        ("g g", CommandId::FirstMessage),
        ("G", CommandId::LastMessage),
        ("Escape", CommandId::Back),
    ] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::List),
            Outcome::Command(expected.as_str().to_owned()),
            "{keys}"
        );
    }
}

#[test]
fn gmails_action_set_works_in_the_list() {
    let mut resolver = resolver();

    for (keys, expected) in [
        ("r", CommandId::Reply),
        ("a", CommandId::ReplyAll),
        ("f", CommandId::Forward),
        ("e", CommandId::Archive),
        ("z", CommandId::Undo),
        ("ctrl+z", CommandId::Undo),
        ("s", CommandId::Flag),
        ("b", CommandId::Snooze),
        ("I", CommandId::ToggleRead),
        ("U", CommandId::ToggleRead),
        ("l", CommandId::AddLabel),
        ("v", CommandId::Move),
        ("#", CommandId::Delete),
        ("Delete", CommandId::Delete),
        ("c", CommandId::Compose),
        ("* a", CommandId::SelectAll),
        ("o", CommandId::OpenMessage),
        ("u", CommandId::PrevView),
    ] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::List),
            Outcome::Command(expected.as_str().to_owned()),
            "{keys}"
        );
    }
}

#[test]
fn the_canvas_application_keys_work() {
    let mut resolver = resolver();

    for (keys, expected) in [
        ("/", CommandId::Search),
        ("ctrl+k", CommandId::CommandPalette),
        ("?", CommandId::CheatSheet),
    ] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::List),
            Outcome::Command(expected.as_str().to_owned()),
            "{keys}"
        );
    }
}

#[test]
fn the_composer_keys_work_and_the_list_keys_do_not() {
    let mut resolver = resolver();

    for (keys, expected) in [
        ("ctrl+Return", CommandId::Send),
        ("ctrl+s", CommandId::SaveDraft),
        ("Escape", CommandId::Back),
    ] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::Composer),
            Outcome::Command(expected.as_str().to_owned()),
            "{keys}"
        );
    }

    for keys in ["e", "E", "c", "j", "?", "/"] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::Composer),
            Outcome::Unhandled,
            "`{keys}` is a character while composing, not a command"
        );
    }

    // #426: reply, reply-all and forward resolve while composing, so `Composer::dispatch` can
    // explain its refusal.
    for (keys, expected) in [
        ("r", CommandId::Reply),
        ("a", CommandId::ReplyAll),
        ("f", CommandId::Forward),
    ] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::Composer),
            Outcome::Command(expected.as_str().to_owned()),
            "`{keys}` while composing must still reach the composer"
        );
    }
}

#[test]
fn typing_a_word_into_the_composer_runs_nothing() {
    let mut resolver = resolver();
    let now = std::time::Instant::now();

    for character in "already archived, deleting"
        .chars()
        .filter(|c| c.is_alphabetic())
    {
        let chord = format!("{character}").parse().expect("a chord");
        assert_eq!(
            resolver.press(&chord, KeyContext::Composer, true, now),
            Outcome::Unhandled,
            "`{character}` fired while the user was typing"
        );
    }
}

#[test]
fn every_registry_binding_resolves_in_every_context_it_claims() {
    let mut resolver = resolver();

    for spec in postio_core::registry::all() {
        if !postio_core::registry::offered_on(spec.id.into(), Platform::Freedesktop) {
            continue;
        }
        for context in Context::ALL {
            if !spec.available_in(*context) {
                continue;
            }
            for binding in spec.bindings().filter(|binding| {
                postio_core::registry::binding_offered_on(
                    spec.id.into(),
                    binding,
                    Platform::Freedesktop,
                )
            }) {
                let binding = &expand_mod(binding, Platform::Freedesktop);
                let outcome = press(&mut resolver, binding, KeyContext::from(*context));
                assert_eq!(
                    outcome,
                    Outcome::Command(spec.id.as_str().to_owned()),
                    "`{binding}` in {context:?} should reach `{}`",
                    spec.id
                );
            }
        }
    }
}

#[test]
fn a_keys_override_reaches_the_resolver_and_frees_the_default() {
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_owned(), "y".to_owned());

    let (keymap, problems) = Keymap::from_commands(&postio_core::Keymap::resolve_on(
        &overrides,
        Platform::Freedesktop,
    ));
    assert!(problems.is_empty(), "{problems:?}");
    let mut resolver = Resolver::new(keymap);

    assert_eq!(
        press(&mut resolver, "y", KeyContext::List),
        Outcome::Command("archive".to_owned())
    );
    assert_eq!(
        press(&mut resolver, "e", KeyContext::List),
        Outcome::Unhandled,
        "the default it replaced is gone, not still lying around"
    );
}

#[test]
fn an_unusable_override_leaves_the_command_its_default() {
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_owned(), "ctrl+".to_owned());

    let resolved = postio_core::Keymap::resolve_on(&overrides, Platform::Freedesktop);
    assert!(
        !resolved.problems().is_empty(),
        "the settings panel has to be able to say why"
    );

    let (keymap, problems) = Keymap::from_commands(&resolved);
    assert!(problems.is_empty(), "{problems:?}");
    let mut resolver = Resolver::new(keymap);

    assert_eq!(
        press(&mut resolver, "e", KeyContext::List),
        Outcome::Command("archive".to_owned()),
        "a broken binding must not cost the command its key"
    );
}

/// #1825: the dropdown's chords must reach their commands while typing, which takes a chord holding
/// Ctrl, Alt or Super.
#[test]
fn the_dropdowns_keys_reach_their_commands_while_typing() {
    let mut resolver = resolver();
    let now = std::time::Instant::now();
    for id in [
        CommandId::ShowAllResults,
        CommandId::ForgetRecent,
        CommandId::ExcludeSuggestion,
    ] {
        let keymap =
            postio_core::Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
        let key = keymap.binding(id).expect("bound on Linux").to_owned();
        let binding: Binding = key.parse().expect("a binding");
        let mut outcome = Outcome::Unhandled;
        for chord in binding.chords() {
            outcome = resolver.press(chord, KeyContext::Search, true, now);
        }
        assert_eq!(
            outcome,
            Outcome::Command(id.as_str().to_owned()),
            "`{key}` does not reach `{id}` from the search field"
        );
    }
}
