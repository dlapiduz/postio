use postio_ffi::{KeyOutcomeFfi, ModifiersFfi, Session, SessionOptions, UiContext};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

const NONE: ModifiersFfi = ModifiersFfi {
    control: false,
    option: false,
    shift: false,
    command: false,
};

/// `mod` expands to ⌘ on Apple, the boundary's platform on every host (`postio_ffi::PLATFORM`);
/// reading the host's let the Linux gate press only Control.
const PRIMARY: ModifiersFfi = ModifiersFfi {
    control: false,
    option: false,
    shift: false,
    command: true,
};

fn typed(
    session: &Session,
    character: &str,
    context: UiContext,
    in_text_entry: bool,
) -> KeyOutcomeFfi {
    session.key(Some(character), None, NONE, context, in_text_entry)
}

#[test]
fn a_bound_key_answers_the_command_it_is_bound_to() {
    let session = session();
    assert_eq!(
        typed(&session, "e", UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "archive".to_string()
        },
        "`e` in the list is archive"
    );
    assert_eq!(
        typed(&session, "z", UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "undo".to_string()
        },
        "`z` in the list is Gmail's undo"
    );
    assert_eq!(
        session.key(Some("z"), None, PRIMARY, UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "undo".to_string()
        }
    );
    session.shutdown();
}

#[test]
fn a_sequence_is_pending_until_its_last_chord() {
    let session = session();
    // `g` alone must neither fire nor be unhandled: the frontend swallows a pending chord so its
    // first key does not reach the widget.
    let first = typed(&session, "g", UiContext::List, false);
    assert_eq!(
        first,
        KeyOutcomeFfi::Pending {
            description: "g".to_string()
        },
        "`g` alone resolved to something"
    );
    assert_eq!(
        typed(&session, "g", UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "first_message".to_string()
        },
        "`g g` did not complete the sequence"
    );
    session.shutdown();
}

#[test]
fn typing_wins_over_a_bare_character_binding() {
    let session = session();
    // A focused search field takes `a`; the list must not archive on it.
    assert_eq!(
        typed(&session, "a", UiContext::Search, true),
        KeyOutcomeFfi::Unhandled,
        "`a` archived mail while somebody was typing the word"
    );
    // A chord with a modifier still fires, or a text field would swallow every shortcut.
    assert_eq!(
        session.key(Some("k"), None, PRIMARY, UiContext::Search, true),
        KeyOutcomeFfi::Command {
            id: "command_palette".to_string()
        },
        "the palette would not open from a focused search field"
    );
    session.shutdown();
}

#[test]
fn the_primary_modifier_is_this_platform_s() {
    let session = session();
    // Every `mod+…` default was unparseable on Apple until #656; this guards that the expansion and
    // the resolver agree.
    assert_eq!(
        session.key(Some("k"), None, PRIMARY, UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "command_palette".to_string()
        },
        "the primary accelerator does not open the palette on this platform"
    );
    session.shutdown();
}

#[test]
fn a_named_key_is_named_rather_than_charactered() {
    let session = session();
    // AppKit spells Escape as U+001B and GDK as `\u{1b}` plus "Escape"; both send the name, and the
    // resolver has one table.
    assert_eq!(
        session.key(None, Some("Escape"), NONE, UiContext::Search, true),
        KeyOutcomeFfi::Command {
            id: "back".to_string()
        },
        "Escape did not get the user out of the search field"
    );
    session.shutdown();
}

#[test]
fn a_key_with_neither_a_character_nor_a_name_is_unhandled() {
    let session = session();
    // A dead key mid-composition must propagate, or non-Latin keyboards lose their composition.
    assert_eq!(
        session.key(None, None, NONE, UiContext::List, false),
        KeyOutcomeFfi::Unhandled
    );
    session.shutdown();
}

#[test]
fn slash_opens_search_from_the_list() {
    // `/` is a bare character with no name, so the frontend sends `character: "/"` and nothing
    // else.
    let session = session();
    assert!(
        matches!(
            session.key(Some("/"), None, NONE, UiContext::List, false),
            KeyOutcomeFfi::Command { .. }
        ),
        "`/` resolved to nothing from the list: {:?}",
        session.key(Some("/"), None, NONE, UiContext::List, false)
    );
}

#[test]
fn slash_is_refused_while_a_field_has_the_keyboard() {
    // Typing `/` into the search field must type a slash, not reopen search; `in_text_entry` is the
    // caller's answer.
    let session = session();
    assert!(
        matches!(
            session.key(Some("/"), None, NONE, UiContext::List, true),
            KeyOutcomeFfi::Unhandled
        ),
        "`/` was swallowed while something was being typed into"
    );
}

#[test]
fn asking_for_a_buttons_key_does_not_resolve_the_keymap_again() {
    // Every move re-rendered the reader's buttons and toolbar, each resolving the whole keymap for
    // a tooltip:
    // about 50 ms of a 100 ms budget. Bindings do not change while a session is open.
    let session = session();
    let _ = session.bindings_for("reply".to_owned());
    let before = postio_core::test_support::keymap_resolutions();

    for command in ["reply", "reply_all", "forward", "archive", "expand_all"] {
        let _ = session.bindings_for(command.to_owned());
    }

    assert_eq!(
        postio_core::test_support::keymap_resolutions() - before,
        0,
        "the keymap is resolved once per session, not once per question"
    );
}

#[test]
fn the_mac_answers_focus_keys_and_not_the_three_pane_apps() {
    let session = session();
    assert_eq!(
        typed(&session, "y", UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "accept_invite".to_string()
        },
        "`y` accepts an invitation in Focus"
    );
    assert_eq!(
        session.key(Some("b"), None, PRIMARY, UiContext::List, false),
        KeyOutcomeFfi::Unhandled,
        "Focus has no sidebar to toggle"
    );
    session.shutdown();
}

/// Asserted for each platform from either host: a gate that runs one platform cannot see the
/// other's answer (31 bindings once died that way).
#[test]
fn focus_s_keymap_resolves_on_both_platforms() {
    use postio_config::paths::Platform;
    for platform in [Platform::Freedesktop, Platform::Apple] {
        let keymap = postio_core::Keymap::resolve_on(&Default::default(), platform);
        assert!(
            keymap.problems().is_empty(),
            "{platform:?}: {:?}",
            keymap.problems()
        );
        let (_, problems) =
            postio_ui::keymap::Resolver::from_commands_for(&keymap, postio_ffi::FRONTEND);
        assert!(problems.is_empty(), "{platform:?}: {problems:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_keys_rebinds_while_the_app_runs() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[keys]\narchive = \"w\"\n").expect("written");
    let session = Session::open(SessionOptions::in_memory().with_config_file_for_test(&path))
        .expect("a session");
    let archive = KeyOutcomeFfi::Command {
        id: "archive".to_string(),
    };
    assert_eq!(typed(&session, "w", UiContext::List, false), archive);

    std::fs::write(&path, "[keys]\narchive = \"a\"\n").expect("rewritten");

    let heard = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match session.next_event().await {
                Some(postio_ffi::UiEvent::KeymapChanged) => return true,
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(heard, "the frontend is told the keymap changed");
    assert_eq!(typed(&session, "a", UiContext::List, false), archive);
    assert_eq!(
        typed(&session, "w", UiContext::List, false),
        KeyOutcomeFfi::Unhandled,
        "the old key is free again"
    );
    session.shutdown();
}
