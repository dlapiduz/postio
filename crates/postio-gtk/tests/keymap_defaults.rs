//! The canvas's default binding set, end to end.
//!
//! The path a key travels is: the command registry in `postio-core` holds the
//! defaults as data, `postio_core::Keymap::resolve` lays `[keys]` over them,
//! and `postio_gtk::keymap::Keymap::from_commands` turns the result into
//! something a key press can be matched against. Each of those has its own
//! tests; what is checked here is that the three of them together produce the
//! bindings the design canvas draws.
//!
//! No display, no GTK main loop: every layer involved is pure.

use postio_config::KeyBindings;
use postio_config::keys::{DEFAULT_BINDINGS, binding_problem, expand_mod};
use postio_config::paths::Platform;
use postio_core::{CommandId, Context};
use postio_gtk::keymap::{Binding, KeyContext, Keymap, Outcome, Resolver};

/// The whole default set, resolved with no user overrides.
fn defaults() -> (Keymap, Vec<String>) {
    Keymap::from_commands(&postio_core::Keymap::resolve(&KeyBindings::default()))
}

fn resolver() -> Resolver {
    let (keymap, problems) = defaults();
    assert!(problems.is_empty(), "{problems:?}");
    Resolver::new(keymap)
}

/// Types a whole binding string and returns what the last press meant.
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

// ---------------------------------------------------------------------------
// The set builds
// ---------------------------------------------------------------------------

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
    // `postio-config` validates `[keys]` in prose for the settings panel and
    // deliberately builds no typed binding; this crate parses for real. Two
    // parsers of one syntax drift unless something holds them together.
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

    // The one place the two parsers are *meant* to disagree. `mod` is valid in
    // a config file and invalid in the resolver, because it is resolved in
    // between: `Keymap::resolve` turns it into `ctrl` here and `cmd` on macOS.
    // Written down so that making them agree reads as the regression it would
    // be -- whichever side got "fixed" would break the other platform.
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
fn every_binding_the_config_crate_documents_parses() {
    for (command, binding) in DEFAULT_BINDINGS {
        // Expanded, because the resolver refuses `mod` on purpose -- it is a
        // config-file word, and `Keymap::resolve` is what turns it into a key.
        let binding = expand_mod(binding, Platform::Freedesktop);
        binding
            .parse::<Binding>()
            .unwrap_or_else(|error| panic!("{command} = {binding}: {error}"));
    }
}

// ---------------------------------------------------------------------------
// Every canvas binding works
// ---------------------------------------------------------------------------

#[test]
fn the_canvas_navigation_set_works_in_the_list() {
    let mut resolver = resolver();

    for (keys, expected) in [
        ("j", CommandId::NextMessage),
        ("k", CommandId::PrevMessage),
        ("h", CommandId::PrevView),
        ("l", CommandId::OpenMessage),
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
fn the_canvas_action_set_works_in_the_list() {
    let mut resolver = resolver();

    for (keys, expected) in [
        ("e", CommandId::Reply),
        ("E", CommandId::ReplyAll),
        ("f", CommandId::Forward),
        ("a", CommandId::Archive),
        ("A", CommandId::ArchiveThread),
        ("u", CommandId::Undo),
        ("t", CommandId::Thread),
        ("s", CommandId::Flag),
        ("m", CommandId::Move),
        ("d", CommandId::Delete),
        ("c", CommandId::Compose),
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

    for keys in ["a", "A", "c", "j", "?", "/"] {
        assert_eq!(
            press(&mut resolver, keys, KeyContext::Composer),
            Outcome::Unhandled,
            "`{keys}` is a character while composing, not a command"
        );
    }

    // #426: reply, reply-all and forward are the one exception -- they *do*
    // resolve while composing, so `Composer::dispatch` gets the chance to
    // explain why it is refusing instead of the key vanishing silently.
    for (keys, expected) in [
        ("e", CommandId::Reply),
        ("E", CommandId::ReplyAll),
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
        for context in Context::ALL {
            if !spec.available_in(*context) {
                continue;
            }
            for binding in spec.bindings() {
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

// ---------------------------------------------------------------------------
// Overridable
// ---------------------------------------------------------------------------

#[test]
fn a_keys_override_reaches_the_resolver_and_frees_the_default() {
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("archive".to_owned(), "y".to_owned());

    let (keymap, problems) = Keymap::from_commands(&postio_core::Keymap::resolve(&overrides));
    assert!(problems.is_empty(), "{problems:?}");
    let mut resolver = Resolver::new(keymap);

    assert_eq!(
        press(&mut resolver, "y", KeyContext::List),
        Outcome::Command("archive".to_owned())
    );
    assert_eq!(
        press(&mut resolver, "a", KeyContext::List),
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

    let resolved = postio_core::Keymap::resolve(&overrides);
    assert!(
        !resolved.problems().is_empty(),
        "the settings panel has to be able to say why"
    );

    let (keymap, problems) = Keymap::from_commands(&resolved);
    assert!(problems.is_empty(), "{problems:?}");
    let mut resolver = Resolver::new(keymap);

    assert_eq!(
        press(&mut resolver, "a", KeyContext::List),
        Outcome::Command("archive".to_owned()),
        "a broken binding must not cost the command its key"
    );
}
