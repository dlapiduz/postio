//! Configuration, as the frontend reads it.

use postio_ffi::{Session, SessionOptions};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn a_command_with_no_override_answers_its_built_in_binding() {
    // What a menu draws when the user has changed nothing. The registry's
    // default binding is already on `commands()`, but a menu must not use it
    // directly — it has to ask, or a rebound key shows the wrong accelerator.
    let session = session();
    let binding = session
        .binding_for("archive".to_string())
        .expect("`archive` has a built-in binding");
    assert!(!binding.is_empty());
    session.shutdown();
}

#[test]
fn an_override_wins_over_the_built_in() {
    // The whole reason this crosses. A menu drawing the registry default for a
    // command the user rebound is confidently wrong, which is worse for a menu
    // item than showing no key at all.
    let session = Session::open(
        SessionOptions::in_memory().with_config_for_test("[keys]\narchive = \"ctrl+shift+a\"\n"),
    )
    .expect("a session with a rebinding");

    assert_eq!(
        session.binding_for("archive".to_string()).as_deref(),
        Some("ctrl+shift+a")
    );
    session.shutdown();
}

#[test]
fn a_command_nobody_has_heard_of_has_no_binding() {
    let session = session();
    assert_eq!(session.binding_for("summon_a_pony".to_string()), None);
    session.shutdown();
}

#[test]
fn an_unreadable_config_does_not_stop_the_session() {
    // A config that will not parse is a reason to use the defaults, not a
    // reason the application cannot open. The store and the mail are not
    // downstream of `[keys]`.
    let session =
        Session::open(SessionOptions::in_memory().with_config_for_test("this is not toml {{{"))
            .expect("a broken config still opens a session");
    assert!(session.binding_for("archive".to_string()).is_some());
    session.shutdown();
}

#[test]
fn the_token_never_crosses_to_swift() {
    // Swift has no way to know which key `mod` means, and deciding that is the
    // core's job -- it owns the binding table. A `KeyboardShortcut` built from
    // the literal token would render a menu item reading "mod+K".
    let session = session();
    for command in ["command_palette", "settings", "bold", "archive", "reply"] {
        if let Some(binding) = session.binding_for(command.to_string()) {
            assert!(
                !binding.contains("mod+"),
                "`{command}` crossed as `{binding}`"
            );
        }
    }
    session.shutdown();
}

#[test]
fn a_non_default_sync_section_actually_reaches_the_engine_this_session_starts() {
    // #1014: `Session::open` used to build every `Wiring` with
    // `BackfillPolicy::default()`/`WatchPolicy::default()` regardless of what
    // `[sync]` said, so this has to prove the engine was actually handed a
    // *different* policy -- not merely that the session opened.
    let text = "[sync]\n\
                check_for_mail = \"manual\"\n\
                poll_interval_secs = 42\n\
                body_fetch = \"eager\"\n";
    let session = Session::open(SessionOptions::in_memory().with_config_for_test(text))
        .expect("a session with a non-default [sync]");

    let expected: postio_config::SyncConfig = postio_config::Config::from_toml_str(text)
        .expect("parses")
        .sync;
    assert_ne!(
        expected,
        postio_config::SyncConfig::default(),
        "the fixture must actually differ from the default, or this proves nothing"
    );
    assert!(
        session.honors_sync_config_for_test(&expected),
        "the wiring's backfill/watch policy does not match what [sync] asked for"
    );
    session.shutdown();
}

#[test]
fn a_file_that_says_nothing_leaves_the_built_in_sync_defaults_standing() {
    let session = session();
    assert!(session.honors_sync_config_for_test(&postio_config::SyncConfig::default()));
    session.shutdown();
}

#[test]
fn a_mod_override_reaches_swift_as_this_platforms_accelerator() {
    // A `config.toml` written on Linux and synced to a Mac. The file says
    // `mod+shift+a` on both machines; the menu says Command here.
    let session = Session::open(
        SessionOptions::in_memory().with_config_for_test("[keys]\narchive = \"mod+shift+a\"\n"),
    )
    .expect("a session with a rebinding");

    let expected = match cfg!(target_os = "macos") {
        true => "cmd+shift+a",
        false => "ctrl+shift+a",
    };
    assert_eq!(
        session.binding_for("archive".to_string()).as_deref(),
        Some(expected)
    );
    session.shutdown();
}
