//! Every command reaches something, or this says which ones do not.
//!
//! The classic app had had this sweep since #756 —
//! the classic app's command-wiring test — and its `KNOWN_ORPHANS` list is empty
//! because the sweep has existed long enough to have emptied it. macOS had
//! nothing of the kind, which is how a build shipped where `refresh` was in
//! the File menu, bound to `F5` and `R`, listed in the palette, and answered
//! by nobody: three surfaces offering a key that did nothing, and a green
//! suite the whole time.
//!
//! **This is the one test in this suite that is about absence.** Everything
//! else here asserts that a thing works; this asserts that nothing was left
//! out, which is the only shape that catches a command *not* wired.
//!
//! A command is answered if one of three things is true:
//!
//! 1. The bus has a handler for it — `Dispatcher::wired`.
//! 2. The Swift frontend presents a surface for it rather than dispatching
//!    it — [`INTERCEPTED`], which is `PostioKit`'s `Intercepted.all`. A
//!    session cannot open a window, so those stop here by design.
//! 3. Focus's controller answers it on the list — `Session::invoke` hands
//!    the controller its own commands before anything else
//!    (specs/009-focus-macos T046), and what it says reaches Swift as
//!    `UiEvent`s.
//!
//! Or it is not offered on the Mac at all — `postio_core::registry::offered_on`,
//! or a requirement Focus on Apple does not meet — because the Mac has no
//! surface for it. Settled rather than answered: no menu item, no key, no
//! palette row, so nothing is drawn that does nothing. That is a decision
//! recorded in the registry, not debt.
//!
//! The Mac is Focus (specs/009-focus-macos), so every command Focus offers is
//! owed here; the ones whose surface a later task builds are `KNOWN_ORPHANS`,
//! each citing that task.
//!
//! Anything else is an orphan.

use postio_core::CommandId;

/// The one list, read from the library rather than written again here.
const INTERCEPTED: &[CommandId] = postio_ffi::registry::INTERCEPTED;

/// Commands nothing answers on macOS yet, each with the task that will.
///
/// **Debt, not permission.** The list may only shrink: a command that gains a
/// handler and is still listed fails the second assertion below, the same way
/// a command that loses one fails the first. That is what stops this becoming
/// a place orphans go to be forgotten -- which is exactly what happened
/// without a sweep at all.
///
/// Every entry is a Focus command whose surface the Mac builds in a later
/// task of specs/009-focus-macos; the task that builds it deletes the line.
const KNOWN_ORPHANS: &[(CommandId, &str)] = &[
    // The message window's raw source and treatment switch.
    (CommandId::ViewSource, "specs/009-focus-macos T070"),
    (CommandId::SwitchTreatment, "specs/009-focus-macos T070"),
    // The message window's More menu.
    (CommandId::MoreActions, "specs/009-focus-macos T067"),
    // The message window's attachments and links.
    (
        CommandId::OpenAttachmentOrLink,
        "specs/009-focus-macos T068",
    ),
    (
        CommandId::ToggleReadingPane,
        "specs/009-focus-macos M4 (the reading pane beside the list comes after parity)",
    ),
    // The Filtered view. `g f` is the controller's now (`FocusShowFiltered`),
    // and the view it shows is T113's.
    (CommandId::FilteredTab1, "specs/009-focus-macos T113"),
    (CommandId::FilteredTab2, "specs/009-focus-macos T113"),
    (CommandId::FilteredTab3, "specs/009-focus-macos T113"),
    (CommandId::FilteredTab4, "specs/009-focus-macos T113"),
    (CommandId::FilteredTab5, "specs/009-focus-macos T113"),
    (CommandId::FilteredTab6, "specs/009-focus-macos T113"),
    (CommandId::FilteredTab7, "specs/009-focus-macos T113"),
    // The digest window.
    (CommandId::GoToDigestRules, "specs/009-focus-macos T114"),
    (CommandId::NextReference, "specs/009-focus-macos T114"),
    (CommandId::PrevReference, "specs/009-focus-macos T114"),
    (CommandId::ToggleDigestSummary, "specs/009-focus-macos T114"),
    // The digest-this-sender sheet.
    (CommandId::DigestRule, "specs/009-focus-macos T115"),
    (CommandId::DigestLikeThis, "specs/009-focus-macos T115"),
    // The capture window.
    (CommandId::CaptureTask, "specs/009-focus-macos T116"),
    (CommandId::CaptureNote, "specs/009-focus-macos T116"),
    (
        CommandId::CaptureChangeProject,
        "specs/009-focus-macos T116",
    ),
    (CommandId::CaptureUseSubject, "specs/009-focus-macos T116"),
    (CommandId::CaptureWrite, "specs/009-focus-macos T116"),
];

/// Whether Focus's controller answers `id` on the Mac's list -- with
/// nothing over it, or with the command bar up: what `Session::invoke`
/// routes to it first.
///
/// The bar counts because the Mac draws it (`CommandBarPanel`, T085): its
/// own keys -- `BackToWords`, the result order, saving -- reach the
/// controller only while it is up, which is the only time they mean
/// anything.
fn the_controller_answers(id: CommandId) -> bool {
    let mut focus = postio_focus::FocusController::new(postio_focus::Policy::for_platform(
        postio_config::paths::Platform::Apple,
    ));
    if focus.answers(id) {
        return true;
    }
    let _ = focus.handle(postio_focus::Input::Command(CommandId::Search));
    focus.answers(id)
}

/// Whether the Mac offers `id` at all. See the module note.
///
/// Focus on Apple: the registry's own question, asked the way the session asks
/// it. Scoped away from the platform (`offered_on`), or requiring what the
/// Mac's frontend is not -- the terminal composer's own verbs
/// (`Requirement::Terminal`) are in no Mac menu, key or palette, so they owe
/// it nothing. A single account is assumed in view, so a command that needs
/// one is owed too.
fn offered_on_the_mac(id: CommandId) -> bool {
    let state = postio_core::Availability {
        scope: postio_core::Scope::Account(postio_model::ids::AccountId::new(1)),
        store_open: true,
        frontend: postio_ffi::FRONTEND,
    };
    postio_core::registry::offered_on(id.into(), postio_config::paths::Platform::Apple)
        && postio_core::registry::get(id).requires.met_by(state)
}

/// The bus the FFI session builds, asked what it answers.
///
/// Composed exactly as `postio_host`'s verbs compose it -- the session is the
/// host's client, and `Host::wired` is what it filters a command by -- so this
/// is the same list a running Postio would check a command against rather
/// than a second opinion about one.
async fn wired() -> Vec<CommandId> {
    let database = postio_storage::test_support::memory().await;
    let state = postio_core::state::SharedState::default();
    let actions = postio_session::actions::Actions::new(database.clone(), state.clone());
    let builder =
        postio_session::actions::wire(postio_core::dispatch::Dispatcher::builder(), actions);
    let builder = postio_session::refresh::wire(
        builder,
        postio_session::refresh::EngineSlot::default(),
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
    // The list is debt and is allowed to shrink, never to drift: an entry
    // that is answered now is a line claiming something untrue about the
    // application, and the next reader believes it.
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
    // A command scoped away from the Mac and then given a handler or a
    // surface there is one of the two decisions being wrong: either the Mac
    // has the surface after all, or somebody built something nothing can
    // reach. Either way the registry line should go.
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
