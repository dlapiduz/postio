use std::path::Path;

use postio_core::{ConfigService, Event};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};
use tempfile::TempDir;

fn write(directory: &Path, body: &str) -> std::path::PathBuf {
    let path = directory.join("config.toml");
    std::fs::write(&path, body).expect("write config.toml");
    path
}

fn press(resolver: &mut Resolver, keys: &str, context: KeyContext) -> Outcome {
    let binding: postio_ui::keymap::Binding = keys
        .parse()
        .unwrap_or_else(|error| panic!("{keys}: {error}"));
    let now = std::time::Instant::now();
    let mut outcome = Outcome::Unhandled;
    for chord in binding.chords() {
        outcome = resolver.press(chord, context, false, now);
    }
    outcome
}

fn command(resolver: &mut Resolver, keys: &str) -> Option<String> {
    match press(resolver, keys, KeyContext::List) {
        Outcome::Command(command) => Some(command),
        _ => None,
    }
}

fn problems(service: &ConfigService) -> String {
    service.keymap().problems().join("\n")
}

#[test]
fn editing_the_keys_section_rebinds_immediately() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\narchive = \"w\"\n");
    let mut service = ConfigService::load(&path);

    let (mut resolver, startup_problems) = Resolver::from_commands(service.keymap());
    assert!(startup_problems.is_empty(), "{startup_problems:?}");
    assert_eq!(command(&mut resolver, "w").as_deref(), Some("archive"));
    assert_eq!(command(&mut resolver, "e"), None, "the default moved");

    write(directory.path(), "[keys]\narchive = \"e\"\nflag = \"w\"\n");
    let update = service.reload();

    assert!(update.applied());
    assert!(update.changed.keys, "the keys section moved");
    let (rebuilt, problems) = Resolver::from_commands(service.keymap());
    assert!(problems.is_empty(), "{problems:?}");
    resolver = rebuilt;

    assert_eq!(
        command(&mut resolver, "e").as_deref(),
        Some("archive"),
        "back to the default, with no restart"
    );
    assert_eq!(command(&mut resolver, "w").as_deref(), Some("flag"));
}

/// `g i` and `/` did nothing from the sidebar: the resolver enforces the registry's contexts, so
/// this asserts here as well as in `core_suite`.
#[test]
fn the_folder_list_reaches_search_and_the_go_to_family() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "");
    let service = ConfigService::load(&path);

    let (mut resolver, problems) = Resolver::from_commands(service.keymap());
    assert!(problems.is_empty(), "{problems:?}");

    assert_eq!(
        press(&mut resolver, "/", KeyContext::Sidebar),
        Outcome::Command("search".to_owned()),
        "`/` opens the box from the folder list, not only from the message list"
    );
    for (keys, command) in [
        ("g i", "go_to_inbox"),
        ("g d", "go_to_drafts"),
        ("g t", "go_to_sent"),
        ("g a", "go_to_archive"),
        ("g b", "go_to_snoozed"),
        ("g s", "go_to_flagged"),
    ] {
        resolver.clear_pending();
        assert_eq!(
            press(&mut resolver, keys, KeyContext::Sidebar),
            Outcome::Command(command.to_owned()),
            "`{keys}` has to reach {command} from the folder list -- standing \
             there is the likeliest moment to want a different folder"
        );
    }
}

#[test]
fn a_rebind_reaches_a_sequence_too() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\nfirst_message = \"g m\"\n");
    let service = ConfigService::load(&path);

    let (mut resolver, problems) = Resolver::from_commands(service.keymap());
    assert!(problems.is_empty(), "{problems:?}");

    assert_eq!(
        command(&mut resolver, "g m").as_deref(),
        Some("first_message")
    );
    assert_eq!(
        press(&mut resolver, "g", KeyContext::List),
        Outcome::Pending("g".to_owned()),
        "the new sequence is pending-aware like any other"
    );
    resolver.clear_pending();
    assert_eq!(command(&mut resolver, "g g"), None, "the old one is gone");
}

#[test]
fn a_rebind_takes_the_key_and_the_command_that_had_it_is_told() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\nreply = \"e\"\n");
    let service = ConfigService::load(&path);

    // The override wins: `[keys]` resolve before defaults ("a default is a suggestion; an override
    // is not"); the loss is reported below.
    let (mut resolver, _) = Resolver::from_commands(service.keymap());
    assert_eq!(
        command(&mut resolver, "e").as_deref(),
        Some("reply"),
        "the override takes the key it asked for"
    );

    let reported = problems(&service);
    assert!(
        reported.contains("archive") && reported.contains("reply"),
        "the command that lost its key has to be named, and by whom: {reported}"
    );

    assert_eq!(
        service.config().keys.overrides().len(),
        1,
        "the override is applied, not discarded"
    );
}

#[test]
fn an_unknown_command_id_is_reported_rather_than_ignored() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\nteleport = \"ctrl+t\"\n");
    let service = ConfigService::load(&path);

    let reported = problems(&service);
    assert!(
        reported.contains("teleport"),
        "a typo in a command id must not vanish silently: {reported}"
    );

    assert_eq!(
        service
            .config()
            .keys
            .overrides()
            .get("teleport")
            .map(String::as_str),
        Some("ctrl+t")
    );

    let (mut resolver, problems) = Resolver::from_commands(service.keymap());
    assert!(problems.contains(&reported), "{problems:?}");
    assert_eq!(
        command(&mut resolver, "e").as_deref(),
        Some("archive"),
        "and every other binding still works"
    );
}

#[test]
fn an_unusable_binding_is_reported_and_costs_nothing() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\narchive = \"ctrl+\"\n");
    let service = ConfigService::load(&path);

    assert!(
        !service.status().is_valid(),
        "the settings panel shows this on the validity line"
    );

    let (mut resolver, _) = Resolver::from_commands(service.keymap());
    assert_eq!(
        command(&mut resolver, "e").as_deref(),
        Some("archive"),
        "a binding nobody can press must not take the key away"
    );
}

#[test]
fn a_broken_file_leaves_the_working_keys_alone() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\narchive = \"y\"\n");
    let mut service = ConfigService::load(&path);
    let (mut resolver, _) = Resolver::from_commands(service.keymap());
    assert_eq!(command(&mut resolver, "y").as_deref(), Some("archive"));

    write(directory.path(), "[keys\narchive = ");
    let update = service.reload();

    assert!(!update.applied(), "the file could not be read");
    assert!(
        update
            .events
            .iter()
            .any(|event| matches!(event, Event::Error { .. })),
        "and the user is told"
    );
    assert!(
        !update.changed.keys,
        "so the resolver is never asked to rebuild"
    );

    assert_eq!(
        command(&mut resolver, "y").as_deref(),
        Some("archive"),
        "the last good keymap stays in force"
    );
    let edit = postio_config::keys::expand_mod("mod+e", postio_config::paths::Platform::host());
    assert_eq!(
        command(&mut resolver, &edit).as_deref(),
        Some("edit_config"),
        "including the key that opens the file to fix it"
    );
}

#[test]
fn a_save_that_changes_nothing_does_not_rebuild_the_keymap() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\narchive = \"y\"\n");
    let mut service = ConfigService::load(&path);

    write(directory.path(), "[keys]\narchive = \"y\"\n");
    let update = service.reload();

    assert!(
        !update.changed.keys,
        "rebinding on every keystroke of an unrelated edit is visible lag"
    );
    assert!(update.events.is_empty());
}
