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
