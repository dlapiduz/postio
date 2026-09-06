//! `[keys]` applies live: edit `config.toml`, press the new key.
//!
//! The path is `ConfigService::reload` → `ConfigChange { keys: true }` →
//! `Resolver::apply_commands`. Everything here drives a real file on disk
//! through the real config service, because the thing being tested is that the
//! pieces are actually joined up — a unit test of either half would pass with
//! the wire cut.
//!
//! No display and no GTK main loop: only the resolver is involved on this side.

use std::path::Path;

use postio_core::{ConfigService, Event};
use postio_gtk::keymap::{KeyContext, Outcome, Resolver};
use tempfile::TempDir;

fn write(directory: &Path, body: &str) -> std::path::PathBuf {
    let path = directory.join("config.toml");
    std::fs::write(&path, body).expect("write config.toml");
    path
}

fn press(resolver: &mut Resolver, keys: &str, context: KeyContext) -> Outcome {
    let binding: postio_gtk::keymap::Binding = keys
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

/// The problems a service reports, joined so a test can look for a phrase.
fn problems(service: &ConfigService) -> String {
    service.keymap().problems().join("\n")
}

#[test]
fn editing_the_keys_section_rebinds_immediately() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\narchive = \"y\"\n");
    let mut service = ConfigService::load(&path);

    let (mut resolver, startup_problems) = Resolver::from_commands(service.keymap());
    assert!(startup_problems.is_empty(), "{startup_problems:?}");
    assert_eq!(command(&mut resolver, "y").as_deref(), Some("archive"));
    assert_eq!(command(&mut resolver, "a"), None, "the default moved");

    // The user edits the file and saves.
    write(directory.path(), "[keys]\narchive = \"a\"\nflag = \"y\"\n");
    let update = service.reload();

    assert!(update.applied());
    assert!(update.changed.keys, "the keys section moved");
    let problems = resolver.apply_commands(service.keymap());
    assert!(problems.is_empty(), "{problems:?}");

    assert_eq!(
        command(&mut resolver, "a").as_deref(),
        Some("archive"),
        "back to the default, with no restart"
    );
    assert_eq!(command(&mut resolver, "y").as_deref(), Some("flag"));
}

#[test]
fn a_rebind_reaches_a_sequence_too() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "[keys]\nfirst_message = \"g t\"\n");
    let service = ConfigService::load(&path);

    let (mut resolver, problems) = Resolver::from_commands(service.keymap());
    assert!(problems.is_empty(), "{problems:?}");

    assert_eq!(
        command(&mut resolver, "g t").as_deref(),
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
    // `a` already archives, in the same contexts reply is reachable in.
    let path = write(directory.path(), "[keys]\nreply = \"a\"\n");
    let service = ConfigService::load(&path);

    // The override wins. `Keymap::resolve_on` resolves `[keys]` before
    // defaults on purpose -- "a default is a suggestion; an override is not" --
    // so a user who wrote this gets what they asked for.
    //
    // This case used to assert the opposite: that the entry was dropped and
    // archive kept `a`. Both rules were live at once and neither was wrong
    // where it ran, because `postio-config` validated the collision away for
    // the 23 commands it had defaults for and the keymap's rule governed the
    // other 56 (#1227). With one default table there is one answer, and it is
    // the documented one -- ignoring an explicit instruction is the worse
    // failure, and the cost of honouring it is paid by the report below rather
    // than in silence.
    let (mut resolver, _) = Resolver::from_commands(service.keymap());
    assert_eq!(
        command(&mut resolver, "a").as_deref(),
        Some("reply"),
        "the override takes the key it asked for"
    );

    // And archive is told, by name, that it lost its default -- the whole
    // reason honouring the override is safe. Silence here would be the bug
    // the old rule was guarding against.
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

    // And it stays in the file: a binding written by a newer Postio survives a
    // downgrade rather than being helpfully deleted.
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
        command(&mut resolver, "a").as_deref(),
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
        command(&mut resolver, "a").as_deref(),
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

    // Saved halfway through an edit.
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
    assert_eq!(
        command(&mut resolver, "ctrl+e").as_deref(),
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

#[test]
fn a_half_typed_sequence_does_not_survive_a_rebind() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = write(directory.path(), "");
    let mut service = ConfigService::load(&path);
    let (mut resolver, _) = Resolver::from_commands(service.keymap());

    assert_eq!(
        press(&mut resolver, "g", KeyContext::List),
        Outcome::Pending("g".to_owned())
    );

    write(directory.path(), "[keys]\nfirst_message = \"g t\"\n");
    assert!(service.reload().changed.keys);
    resolver.apply_commands(service.keymap());

    assert_eq!(
        resolver.pending(),
        None,
        "it was typed against a table that no longer exists"
    );
}
