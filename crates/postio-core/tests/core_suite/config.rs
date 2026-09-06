//! Configuration, as core consumes it.
//!
//! `config.toml` *is* the settings — the design promises *applied live ·
//! nothing to save* — so core owns the live handle, resolves `[keys]` onto the
//! command registry, and re-emits what changed. Two properties matter most: a
//! broken file must leave the user with the bindings they had a moment ago, and
//! a save that changes nothing anybody cares about must not repaint anything.

use std::time::{Duration, Instant};

use postio_config::paths::Platform;
use postio_config::{Config, KeyBindings, validate};
use postio_core::ActionId;
use postio_core::bridge::event_channel;
use postio_core::config::{ConfigService, Keymap, SharedConfig};
use postio_core::{CommandId, Context, Event, registry};

/// How long a wait on the event stream may go silent before it is called a
/// hang.
///
/// A *liveness* bound, not a latency claim — the waits below are already
/// event-driven, so this deadline does no measuring; it only turns a genuine
/// hang into a failure with a name. It is deliberately enormous: at 10
/// seconds it doubled as a performance budget for inotify delivery plus the
/// watcher's debounce, and a box with four sessions compiling walked through
/// it once (#219). The engineering-notes "tests that fail under load"
/// doctrine is the long form.
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

// -- Resolving [keys] onto the registry --------------------------------------

/// An override landing on a default's key is reported, for any of the 79.
///
/// The half of #1227 that `postio-config`'s validator gave up when its own
/// default table went: it could only see collisions between two `[keys]`
/// entries, and against defaults it knew 23 commands. This is the complete
/// answer, and it has to stay complete — `flag` is one of the 56 the old table
/// had never heard of.
#[test]
fn an_override_that_takes_a_default_key_is_reported() {
    for (taker, victim, key) in [
        (CommandId::Reply, "archive", "a"),
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

/// Every default the registry ships expands to something the validator will
/// take, on both platforms.
///
/// Moved here from `postio-config` with the default table itself (#1227).
/// There it ran over 23 commands; here it runs over all of them, which is the
/// point — `cmd` only ever appears as expansion *output*, so nothing else
/// would catch it going missing from `MODIFIERS`, and the failure mode is a
/// Mac on which every default is rejected as unusable.
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

/// The accelerators a Linux user has today, spelled out.
///
/// Also moved from `postio-config` (#1227), and it has to ask the keymap
/// rather than the config crate now: `mod+k` is the registry's, and the file
/// says nothing. A change here is a change to what somebody's fingers already
/// know, so it should cost a deliberate edit.
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
    let keymap = Keymap::resolve(&KeyBindings::default());

    assert_eq!(keymap.binding(CommandId::Archive), Some("a"));
    assert_eq!(keymap.binding(CommandId::Undo), Some("u"));
    assert!(keymap.problems().is_empty());

    for command in CommandId::ALL {
        assert!(
            keymap.binding(*command).is_some(),
            "`{command}` lost its binding"
        );
    }
}

#[test]
fn an_override_rebinds_the_command_and_keeps_its_alternates() {
    let keymap = Keymap::resolve(&bindings(&[("open_message", "o")]));

    assert_eq!(keymap.binding(CommandId::OpenMessage), Some("o"));
    assert!(
        keymap
            .bindings(CommandId::OpenMessage)
            .contains(&"l".into()),
        "the registry's alternates are not the file's to replace"
    );
    assert_eq!(
        keymap.command_for(Context::List, "o"),
        Some(ActionId::Builtin(CommandId::OpenMessage))
    );
    assert_eq!(
        keymap.command_for(Context::List, "Return"),
        None,
        "the old key is free again"
    );
}

#[test]
fn a_key_resolves_only_in_the_contexts_its_command_lives_in() {
    // Named rather than host-derived: this asserts an accelerator, and since
    // #669 the accelerator for the palette differs by platform. Left as
    // `resolve` it passed on Linux and failed on a Mac.
    let keymap = Keymap::resolve_on(&KeyBindings::default(), Platform::Freedesktop);

    assert_eq!(
        keymap.command_for(Context::List, "a"),
        Some(ActionId::Builtin(CommandId::Archive))
    );
    assert_eq!(
        keymap.command_for(Context::Composer, "a"),
        None,
        "`a` is a letter while composing"
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
    // `postio-config` rejects the file outright for this, but the keymap must
    // not fall over if one ever reaches it.
    let keymap = Keymap::resolve(&bindings(&[("archive", "ctrl+")]));

    assert_eq!(keymap.binding(CommandId::Archive), Some("a"));
    let problem = keymap.problems().first().expect("a problem was reported");
    assert!(problem.contains("archive"), "{problem}");
}

#[test]
fn an_override_for_a_command_this_build_does_not_know_is_only_a_warning() {
    let keymap = Keymap::resolve(&bindings(&[("summarize_thread", "ctrl+i")]));

    assert_eq!(keymap.binding(CommandId::Archive), Some("a"));
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
    // `a` is Archive's default. Someone who writes `delete = "a"` has said
    // what they want, and a default is only a suggestion — so the override
    // takes the key and Archive gives it up rather than the line being
    // ignored.
    //
    // This reverses an earlier rule (registry order wins, the override is
    // dropped). That rule was there so a key could never be silently
    // shadowed, and nothing here is silent: the command that lost its key
    // says so in `problems()`, which the settings panel shows. What the old
    // rule did instead was ignore a line the user had written, and make which
    // of two commands won depend on the order of a table they have never
    // seen — so adding a command with a popular default could quietly take a
    // key somebody was already using. See `postio-7bc`.
    let keymap = Keymap::resolve(&bindings(&[("delete", "a")]));

    assert_eq!(
        keymap.command_for(Context::List, "a"),
        Some(ActionId::Builtin(CommandId::Delete))
    );
    assert_eq!(
        keymap.binding(CommandId::Archive),
        None,
        "and the command that lost the key is palette-only rather than dead"
    );
    assert!(
        keymap
            .problems()
            .iter()
            .any(|problem| problem.contains("archive")),
        "{:?}",
        keymap.problems()
    );
}

#[test]
fn two_overrides_wanting_one_key_are_settled_by_registry_order() {
    // Between two explicit choices there is nothing to prefer, so the order
    // is at least deterministic, and the one that loses is told.
    let keymap = Keymap::resolve(&bindings(&[("archive", "q"), ("delete", "q")]));

    assert_eq!(
        keymap.command_for(Context::List, "q"),
        Some(ActionId::Builtin(CommandId::Archive))
    );
    assert_eq!(
        keymap.binding(CommandId::Delete),
        Some("d"),
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

// -- Live reload -------------------------------------------------------------

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
        [Event::Error { message }] => assert!(!message.is_empty(), "the user is told why"),
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
    // An unknown top-level key round-trips rather than being dropped, but
    // nothing in the application is waiting on it.
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

// -- Watching the file --------------------------------------------------------

#[test]
fn a_save_in_an_external_editor_lands_without_a_restart() {
    // `Ctrl+E` opens $EDITOR; whatever happens in there has to arrive here.
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

    // The editor's save, rename dance and all.
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
        std::thread::sleep(Duration::from_millis(10));
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
