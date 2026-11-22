//! Every command is wired, intercepted by Swift, answered by Focus's controller, or not offered on
//! the Mac; anything else is an orphan.

use postio_core::CommandId;

const INTERCEPTED: &[CommandId] = postio_ffi::registry::INTERCEPTED;

/// The list may only shrink: a command that gains a handler and is still listed fails the second
/// assertion.
const KNOWN_ORPHANS: &[(CommandId, &str)] = &[
    (CommandId::ViewSource, "specs/009-focus-macos T070"),
    (CommandId::SwitchTreatment, "specs/009-focus-macos T070"),
    (CommandId::MoreActions, "specs/009-focus-macos T067"),
    (
        CommandId::ToggleReadingPane,
        "specs/009-focus-macos M4 (the reading pane beside the list comes after parity)",
    ),
    (CommandId::GoToDigestRules, "specs/009-focus-macos T114"),
    (CommandId::DigestLikeThis, "specs/009-focus-macos T115"),
    (
        CommandId::ContactsToggleEveryone,
        "specs/011-focus-contacts",
    ),
    (CommandId::ContactsToggleDeleted, "specs/011-focus-contacts"),
    (CommandId::FirstInConversation, "specs/013-focus-threads"),
    (CommandId::LatestInConversation, "specs/013-focus-threads"),
    (
        CommandId::FirstUnreadInConversation,
        "specs/013-focus-threads",
    ),
    (CommandId::OutlineFirstUnread, "specs/013-focus-threads"),
    (CommandId::ForwardWhat1, "specs/013-focus-threads"),
    (CommandId::ForwardWhat2, "specs/013-focus-threads"),
    (CommandId::ForwardWhat3, "specs/013-focus-threads"),
    (CommandId::ShowNewer, "specs/013-focus-threads"),
    (CommandId::ExpandQuoted, "specs/013-focus-threads"),
];

/// Whether Focus's controller answers `id`: on the list, with the command bar up, or over a surface
/// that keeps its keys only while it is up.
fn the_controller_answers(id: CommandId) -> bool {
    use postio_focus::{FocusController, Input, Policy, SurfaceKind};
    let fresh =
        || FocusController::new(Policy::for_platform(postio_config::paths::Platform::Apple));
    let with_bar = || {
        let mut focus = fresh();
        let _ = focus.handle(Input::Command(CommandId::Search));
        focus.answers(id)
    };
    let with_results = || {
        let mut focus = fresh();
        let _ = focus.handle(Input::Command(CommandId::Search));
        let _ = focus.handle(Input::Typed {
            text: "atlas".to_owned(),
        });
        let _ = focus.handle(Input::Command(CommandId::ShowAllResults));
        focus.answers(id)
    };
    fresh().answers(id)
        || with_bar()
        || with_results()
        || [
            SurfaceKind::Filtered,
            SurfaceKind::Digest,
            SurfaceKind::Capture,
            SurfaceKind::ContactPopover,
            SurfaceKind::Contacts,
        ]
        .into_iter()
        .any(|kind| {
            let mut focus = fresh();
            let _ = focus.handle(Input::SurfaceOpened(kind));
            focus.answers(id)
        })
}

/// Scoped away from the platform, or requiring what the Mac is not (the terminal composer), means
/// not owed.
fn offered_on_the_mac(id: CommandId) -> bool {
    let state = postio_core::Availability {
        scope: postio_core::Scope::Account(postio_model::ids::AccountId::new(1)),
        store_open: true,
        frontend: postio_ffi::FRONTEND,
    };
    postio_core::registry::offered_on(id.into(), postio_config::paths::Platform::Apple)
        && postio_core::registry::get(id).requires.met_by(state)
}

/// Composed as `postio_host` composes it, so this is the list a running Postio checks.
async fn wired() -> Vec<CommandId> {
    let database = postio_storage::test_support::memory().await;
    let state = postio_core::state::SharedState::default();
    let actions = postio_session::actions::Actions::new(database.clone(), state.clone());
    let builder =
        postio_session::actions::wire(postio_core::dispatch::Dispatcher::builder(), actions);
    let builder = postio_session::refresh::wire(
        builder,
        postio_session::refresh::EngineSlot::default(),
        database,
        state,
    );
    builder.build().wired().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_command_reaches_a_handler_a_window_or_this_boundary() {
    let wired = wired().await;
    let known: Vec<CommandId> = KNOWN_ORPHANS.iter().map(|(id, _)| *id).collect();

    let orphans: Vec<CommandId> = CommandId::ALL
        .iter()
        .copied()
        .filter(|id| {
            !wired.contains(id)
                && !INTERCEPTED.contains(id)
                && !the_controller_answers(*id)
                && !known.contains(id)
                && offered_on_the_mac(*id)
        })
        .collect();

    assert!(
        orphans.is_empty(),
        "these commands reach nothing on macOS, so the menu item, the key and \
         the palette entry all do nothing: {orphans:?}. Wire each one in \
         `postio_session::actions` (or `refresh`), answer it in \
         the controller, give the Swift frontend a surface for it and add it to `Intercepted`, or -- \
         if it is genuinely not built yet -- put it in `KNOWN_ORPHANS` with \
         the issue that will build it."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_that_gained_a_handler_leaves_the_orphan_list() {
    // An entry that is answered now claims something untrue; the list may only shrink.
    let wired = wired().await;
    for &(id, issue) in KNOWN_ORPHANS {
        let answered = wired.contains(&id)
            || INTERCEPTED.contains(&id)
            || the_controller_answers(id)
            || !offered_on_the_mac(id);
        assert!(
            !answered,
            "{id} is in KNOWN_ORPHANS citing {issue}, and it is answered now \
             -- delete the line so this sweep keeps meaning something"
        );
    }
}

#[test]
fn nothing_the_mac_does_not_offer_is_also_answered_on_it() {
    // A command scoped away from the Mac but given a handler or surface means one of the two
    // decisions is wrong.
    for id in CommandId::ALL
        .iter()
        .copied()
        .filter(|id| !offered_on_the_mac(*id))
    {
        assert!(
            !INTERCEPTED.contains(&id),
            "`{id}` is not offered on the Mac and is answered there anyway"
        );
    }
}
