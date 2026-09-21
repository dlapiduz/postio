//! Every command reaches something, or this says which ones do not.
//!
//! `postio-gtk` has had this sweep since #756 —
//! `app_suite/command_wiring.rs` — and its `KNOWN_ORPHANS` list is empty
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
//! 1. The boundary handles it itself — [`postio_ffi::HANDLED_HERE`], the
//!    cursor and selection verbs, which move state that lives on this side.
//! 2. The bus has a handler for it — `Dispatcher::wired`.
//! 3. The Swift frontend presents a surface for it rather than dispatching
//!    it — [`INTERCEPTED`], which is `PostioKit`'s `Intercepted.all`. A
//!    session cannot open a window, so those stop here by design.
//!
//! Anything else is an orphan.

use postio_core::CommandId;

/// The one list, read from the library rather than written again here.
const INTERCEPTED: &[CommandId] = postio_ffi::registry::INTERCEPTED;

/// Commands nothing answers on macOS yet, each with the issue that will.
///
/// **Debt, not permission.** The list may only shrink: a command that gains a
/// handler and is still listed fails the second assertion below, the same way
/// a command that loses one fails the first. That is what stops this becoming
/// a place orphans go to be forgotten — which is exactly what happened
/// without a sweep at all.
const KNOWN_ORPHANS: &[(CommandId, &str)] = {
    use CommandId as C;
    &[
        // #1571 -- the composer's verbs. The composer window exists and its
        // toolbar acts; nothing subscribes it to the command stream, which is
        // what `postio-gtk`'s `connect_command` does on the other side.
        (C::ScheduleSend, "#1571"),
        (C::DetachComposer, "#1571"),
        (C::InsertImage, "#1571"),
        // #1573 -- the sidebar's keyboard. `focus_sidebar` moves the keyboard
        // in and then nothing walks.
        (C::NextScope, "#1573"),
        // #1574 -- a query cannot be kept, and the affordance that says it
        // can is drawn enabled.
        (C::SaveSearch, "#1574"),
        (C::RenameSavedSearch, "#1574"),
        (C::MoveSavedSearchUp, "#1574"),
        (C::MoveSavedSearchDown, "#1574"),
        (C::DeleteSavedSearch, "#1574"),
        // #1575 -- the accounts pane's verbs have buttons and no commands.
        (C::AddAccount, "#1575"),
        (C::EditConfig, "#1575"),
        (C::ToggleAccountEnabled, "#1575"),
        (C::RemoveAccount, "#1575"),
        (C::UpdateCredential, "#1575"),
        (C::RebuildAccountIndex, "#1575"),
        (C::SetDefaultAccount, "#1575"),
        // #1576 -- the reading pane, including the most-used key in a mail
        // client: `space` does not turn the page.
        (C::ToggleRail, "#1576"),
        (C::ToggleResultOrder, "#1576"),
    ]
};

/// The bus the FFI session builds, asked what it answers.
///
/// Composed exactly as `DeferredBus::arm` composes it, so this is the same
/// list a running Postio would check a command against rather than a second
/// opinion about one.
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
                && !postio_ffi::HANDLED_HERE.contains(id)
                && !INTERCEPTED.contains(id)
                && !known.contains(id)
        })
        .collect();

    assert!(
        orphans.is_empty(),
        "these commands reach nothing on macOS, so the menu item, the key and \
         the palette entry all do nothing: {orphans:?}. Wire each one in \
         `postio_session::actions` (or `refresh`), answer it in \
         `Session::handle_locally` and list it in `HANDLED_HERE`, give the \
         Swift frontend a surface for it and add it to `Intercepted`, or -- \
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
            || postio_ffi::HANDLED_HERE.contains(&id)
            || INTERCEPTED.contains(&id);
        assert!(
            !answered,
            "{id} is in KNOWN_ORPHANS citing {issue}, and it is answered now \
             -- delete the line so this sweep keeps meaning something"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_answered_twice_by_the_boundary_and_the_bus() {
    // `handle_locally` returns before the send, so an id in both places is
    // one the bus can never see -- a handler that was written, is tested on
    // its own, and is unreachable through the application.
    let wired = wired().await;
    let both: Vec<CommandId> = postio_ffi::HANDLED_HERE
        .iter()
        .copied()
        .filter(|id| wired.contains(id))
        .collect();
    assert!(
        both.is_empty(),
        "the boundary answers these before the bus can, so the bus's handler \
         is unreachable: {both:?}"
    );
}
