use std::time::{Duration, Instant};

use postio_config::paths::Platform;
use postio_config::{Config, KeyBindings, validate};
use postio_core::ActionId;
use postio_core::bridge::event_channel;
use postio_core::config::{ConfigService, Keymap, SharedConfig};
use postio_core::{CommandId, Context, Event, registry};

/// A liveness bound, not a latency claim: deliberately enormous, since a tight deadline was walked
/// through under load (#219).
const PATIENCE: Duration = Duration::from_secs(120);

fn bindings(overrides: &[(&str, &str)]) -> KeyBindings {
    let mut keys = KeyBindings::default();
    for (command, binding) in overrides {
        keys.overrides_mut()
            .insert((*command).to_string(), (*binding).to_string());
    }
    keys
}

fn service_with(toml: &str) -> ConfigService {
    let mut service = ConfigService::new(std::path::Path::new("config.toml"));
    let update = service.apply(validate::check_str(toml));
    assert!(
        update.applied(),
        "the fixture should be valid: {:?}",
        service.status().status()
    );
    service
}

#[test]
fn an_override_that_takes_a_default_key_is_reported() {
    for (taker, victim, key) in [
        (CommandId::Reply, "archive", "e"),
        (CommandId::Reply, "flag", "s"),
    ] {
        let mut overrides = KeyBindings::default();
        overrides
            .overrides_mut()
            .insert(taker.as_str().to_owned(), key.to_owned());
        let keymap = Keymap::resolve_on(&overrides, Platform::Freedesktop);

        assert_eq!(keymap.binding(taker), Some(key));
        let said = keymap
            .problems()
            .iter()
            .any(|problem| problem.contains(victim) && problem.contains(key));
        assert!(
            said,
            "taking `{key}` from `{victim}` was not reported: {:?}",
            keymap.problems()
        );
    }
}

#[test]
fn every_registry_default_expands_to_something_the_validator_accepts() {
    for platform in [Platform::Freedesktop, Platform::Apple] {
        for spec in registry::all() {
            for binding in spec.bindings() {
                let expanded = postio_config::keys::expand_mod(binding, platform);
                assert_eq!(
                    postio_config::keys::binding_problem(&expanded),
                    None,
                    "{} expands to `{expanded}` on {platform:?}, which the validator rejects",
                    spec.id
                );
            }
        }
    }
}

#[test]
fn the_mod_defaults_are_control_on_linux_and_command_on_apple() {
    let linux = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);
    for (command, want) in [
        (CommandId::CommandPalette, "ctrl+k"),
        (CommandId::Settings, "ctrl+comma"),
        (CommandId::AddAccount, "ctrl+shift+n"),
        (CommandId::Bold, "ctrl+b"),
        (CommandId::InsertLink, "ctrl+shift+k"),
    ] {
        assert_eq!(
            linux.binding(command),
            Some(want),
            "{command} changed for Linux users"
        );
    }

    let apple = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    assert_eq!(apple.binding(CommandId::CommandPalette), Some("cmd+k"));
}

#[test]
fn without_a_file_the_registry_defaults_are_the_keymap() {
    for platform in [
        postio_config::paths::Platform::Freedesktop,
        postio_config::paths::Platform::Apple,
    ] {
        let keymap = Keymap::resolve_on(&KeyBindings::default(), platform);

        assert_eq!(keymap.binding(CommandId::Archive), Some("e"));
        assert_eq!(keymap.binding(CommandId::Undo), Some("z"));
        assert!(
            keymap
                .bindings(CommandId::Undo)
                .contains(&postio_config::keys::expand_mod("mod+z", platform)),
            "{platform:?}: undo keeps the chord every other application taught"
        );
        assert!(
            keymap.problems().is_empty(),
            "{platform:?}: {:?}",
            keymap.problems()
        );

        for command in CommandId::ALL {
            let offered = postio_core::registry::offered_on((*command).into(), platform);
            assert_eq!(
                keymap.binding(*command).is_some(),
                offered,
                "`{command}` on {platform:?}: bound {}, offered {offered}",
                keymap.binding(*command).is_some()
            );
        }
    }
}

#[test]
fn an_override_rebinds_the_command_and_keeps_its_alternates() {
    let keymap = Keymap::resolve(&bindings(&[("open_message", "w")]));

    assert_eq!(keymap.binding(CommandId::OpenMessage), Some("w"));
    assert!(
        keymap
            .bindings(CommandId::OpenMessage)
            .contains(&"o".into()),
        "the registry's alternates are not the file's to replace"
    );
    assert_eq!(
        keymap.command_for(Context::List, "w"),
        Some(ActionId::Builtin(CommandId::OpenMessage))
    );
    assert_eq!(
        keymap.command_for(Context::List, "Return"),
        None,
        "the old key is free again"
    );
}

#[test]
fn a_leftover_archive_thread_binding_is_reported_and_ignored() {
    let keymap = Keymap::resolve_on(
        &bindings(&[("archive_thread", "E"), ("archive", "w")]),
        Platform::Freedesktop,
    );
    assert_eq!(
        keymap.problems(),
        ["`archive_thread` is not a command in this build"]
    );
    assert_eq!(keymap.binding(CommandId::Archive), Some("w"));
    assert_eq!(
        keymap.command_for(Context::List, "E"),
        None,
        "the leftover key binds nothing"
    );
}

#[test]
fn a_key_resolves_only_in_the_contexts_its_command_lives_in() {
    // Named platform, since the palette accelerator differs by platform.
    let keymap = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);

    assert_eq!(
        keymap.command_for(Context::List, "e"),
        Some(ActionId::Builtin(CommandId::Archive))
    );
    assert_eq!(
        keymap.command_for(Context::Composer, "e"),
        None,
        "`e` is a letter while composing"
    );
    assert_eq!(
        keymap.command_for(Context::Composer, "ctrl+k"),
        Some(ActionId::Builtin(CommandId::CommandPalette)),
        "the palette is reachable everywhere"
    );
}

#[test]
fn the_palette_is_command_k_on_a_mac_and_control_k_does_not_reach_it() {
    let keymap = Keymap::resolve_on(&KeyBindings::default(), Platform::Apple);
    assert_eq!(
        keymap.command_for(Context::Composer, "cmd+k"),
        Some(ActionId::Builtin(CommandId::CommandPalette))
    );
    assert_eq!(
        keymap.command_for(Context::Composer, "ctrl+k"),
        None,
        "Control is a different key on macOS and must not shadow the palette"
    );
}

#[test]
fn an_unusable_binding_falls_back_to_the_default_and_says_why() {
    let keymap = Keymap::resolve(&bindings(&[("archive", "ctrl+")]));

    assert_eq!(keymap.binding(CommandId::Archive), Some("e"));
    let problem = keymap.problems().first().expect("a problem was reported");
    assert!(problem.contains("archive"), "{problem}");
}

#[test]
fn an_override_for_a_command_this_build_does_not_know_is_only_a_warning() {
    let keymap = Keymap::resolve(&bindings(&[("summarize_thread", "ctrl+i")]));

    assert_eq!(keymap.binding(CommandId::Archive), Some("e"));
    assert_eq!(keymap.command_for(Context::List, "ctrl+i"), None);
    assert!(
        keymap
            .problems()
            .iter()
            .any(|problem| problem.contains("summarize_thread")),
        "{:?}",
        keymap.problems()
    );
}

#[test]
fn an_override_takes_a_key_from_the_default_that_had_it() {
    // A default is only a suggestion: the override takes `e` and Archive says so in `problems()`
    // (`postio-7bc`).
    let keymap = Keymap::resolve(&bindings(&[("delete", "e")]));

    assert_eq!(
        keymap.command_for(Context::List, "e"),
        Some(ActionId::Builtin(CommandId::Delete))
    );
    assert_ne!(
        keymap.binding(CommandId::Archive),
        Some("e"),
        "the override took the key"
    );
    assert!(
        keymap
            .binding(CommandId::Archive)
            .is_some_and(|binding| binding.ends_with("shift+a")),
        "and what it keeps is the chord, which is not what was taken"
    );

    let keymap = Keymap::resolve(&bindings(&[("flag", "b")]));
    assert_eq!(
        keymap.binding(CommandId::Snooze),
        None,
        "the command that lost its only key is palette-only rather than dead"
    );
    assert!(
        keymap
            .problems()
            .iter()
            .any(|problem| problem.contains("snooze")),
        "{:?}",
        keymap.problems()
    );
}

#[test]
fn two_overrides_wanting_one_key_are_settled_by_registry_order() {
    let keymap = Keymap::resolve_on(
        &bindings(&[("archive", "q"), ("delete", "q")]),
        postio_config::paths::Platform::Freedesktop,
    );

    assert_eq!(
        keymap.command_for(Context::List, "q"),
        Some(ActionId::Builtin(CommandId::Archive))
    );
    assert_eq!(
        keymap.binding(CommandId::Delete),
        Some("#"),
        "the loser falls back to its own default, which nobody took"
    );
    assert!(
        keymap
            .problems()
            .iter()
            .any(|problem| problem.contains("delete")),
        "{:?}",
        keymap.problems()
    );
}

#[test]
fn editing_keys_rebinds_without_a_restart() {
    let mut service = service_with("[keys]\narchive = \"x\"\n");
    assert_eq!(service.keymap().binding(CommandId::Archive), Some("x"));

    let update = service.apply(validate::check_str("[keys]\narchive = \"z\"\n"));

    assert!(update.applied());
    assert!(update.changed.keys, "the keys section changed");
    assert!(!update.changed.ui, "and nothing else did");
    assert_eq!(service.keymap().binding(CommandId::Archive), Some("z"));
    assert!(
        update
            .events
            .iter()
            .any(|event| matches!(event, Event::ConfigReloaded { .. })),
        "{:?}",
        update.events
    );
}

#[test]
fn a_broken_file_keeps_the_last_good_bindings() {
    let mut service = service_with("[keys]\narchive = \"x\"\n");

    let update = service.apply(validate::check_str("[keys\narchive = \"z\"\n"));

    assert!(!update.applied());
    assert_eq!(
        service.keymap().binding(CommandId::Archive),
        Some("x"),
        "the last good binding stayed in force"
    );
    match update.events.as_slice() {
        [Event::Error { message, .. }] => assert!(!message.is_empty(), "the user is told why"),
        other => panic!("expected one error, got {other:?}"),
    }
}

#[test]
fn rewriting_the_file_with_the_same_content_says_nothing() {
    let toml = "[ui]\ndensity = \"compact\"\n";
    let mut service = service_with(toml);

    let update = service.apply(validate::check_str(toml));

    assert!(!update.applied());
    assert!(update.events.is_empty(), "{:?}", update.events);
    assert!(!update.changed.any());
}

#[test]
fn only_the_sections_that_changed_are_reported() {
    let mut service = service_with("[ui]\ndensity = \"compact\"\n[keys]\narchive = \"x\"\n");

    let update = service.apply(validate::check_str(
        "[ui]\ndensity = \"comfortable\"\n[keys]\narchive = \"x\"\n",
    ));

    assert!(update.changed.ui);
    assert!(!update.changed.keys, "the keymap need not be rebuilt");
    assert!(!update.changed.sync);
    assert!(!update.changed.filters);
}

#[test]
fn a_change_no_subsystem_cares_about_repaints_nothing() {
    let mut service = service_with("[keys]\narchive = \"x\"\n");

    let update = service.apply(validate::check_str(
        "written_by = \"a newer postio\"\n[keys]\narchive = \"x\"\n",
    ));

    assert!(update.applied(), "the file did change");
    assert!(!update.changed.any());
    assert!(update.events.is_empty(), "{:?}", update.events);
}

#[test]
fn the_config_in_force_is_readable_for_the_subsystems_that_want_it() {
    let service = service_with("[sync]\nidle = false\n[ui]\ndensity = \"compact\"\n");

    let config: &Config = service.config();
    assert_eq!(
        config.sync.check_for_mail,
        postio_config::CheckForMail::Poll
    );
    assert_eq!(config.ui.density, postio_config::Density::Compact);
}

#[test]
fn a_save_in_an_external_editor_lands_without_a_restart() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[keys]\narchive = \"x\"\n").expect("write the fixture");

    let config = SharedConfig::load(&path);
    assert_eq!(
        config.read(|service| service
            .keymap()
            .binding(CommandId::Archive)
            .map(str::to_owned)),
        Some("x".to_string())
    );

    let (events, stream) = event_channel();
    let watcher = config.watch(events).expect("the watcher starts");

    std::fs::write(&path, "[keys]\narchive = \"z\"\n").expect("save from the editor");

    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(event) = stream.try_next() {
            if matches!(event, Event::ConfigReloaded { .. }) {
                break;
            }
            continue;
        }
        assert!(Instant::now() < deadline, "no reload within {PATIENCE:?}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert_eq!(
        config.read(|service| service
            .keymap()
            .binding(CommandId::Archive)
            .map(str::to_owned)),
        Some("z".to_string()),
        "the new binding is in force"
    );
    drop(watcher);
}

#[test]
fn the_watcher_reports_which_sections_moved() {
    let service = service_with("[ui]\ndensity = \"compact\"\n");
    let shared = SharedConfig::new(service);

    let (events, stream) = event_channel();
    shared.apply(
        validate::check_str("[ui]\ndensity = \"comfortable\"\n"),
        &events,
    );

    match stream.try_next() {
        Some(Event::ConfigReloaded { changed }) => {
            assert!(changed.ui);
            assert!(!changed.keys);
        }
        other => panic!("expected a reload, got {other:?}"),
    }
}

#[test]
fn the_registry_defaults_are_resolved_once_for_the_whole_process() {
    // Resolution is quadratic in the number of commands; a widget wanting the defaults must not pay
    // per instance (#1216).
    assert!(std::ptr::eq(Keymap::defaults(), Keymap::defaults()));
    assert_eq!(
        Keymap::defaults(),
        &Keymap::resolve(&KeyBindings::default())
    );
}

#[test]
fn asking_for_the_defaults_repeatedly_resolves_nothing() {
    let _ = Keymap::defaults();
    let before = postio_core::test_support::keymap_resolutions();
    for _ in 0..200 {
        let _ = Keymap::defaults();
    }
    assert_eq!(postio_core::test_support::keymap_resolutions(), before);
}
