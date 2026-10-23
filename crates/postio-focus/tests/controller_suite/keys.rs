//! Who runs a key while a surface is over the list (specs/009-focus-macos
//! T054, T055): the table a frontend that keeps the widgets of some
//! surfaces itself reads, over the surface stack.

use crate::search_support::shown;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{FocusController, Input, Intent, Policy, Route, SurfaceKind};
use postio_ui::keymap::KeyContext;

fn linux() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Freedesktop))
}

fn with(kind: SurfaceKind) -> FocusController {
    let mut controller = linux();
    controller.handle(Input::SurfaceOpened(kind));
    controller
}

#[test]
fn the_list_owns_its_keys_with_nothing_over_it() {
    let controller = linux();
    assert_eq!(controller.top_surface(), None);
    assert_eq!(controller.route(CommandId::Archive), Route::List);
    assert_eq!(controller.route(CommandId::Back), Route::List);
}

#[test]
fn the_open_message_owns_back_and_its_reader_verbs_and_leaves_the_rest_to_the_list() {
    let controller = with(SurfaceKind::Message);
    assert_eq!(controller.key_context(), KeyContext::Reader);
    for id in [
        CommandId::Back,
        CommandId::FindInMessage,
        CommandId::FindNext,
        CommandId::FindPrevious,
        CommandId::SwitchTreatment,
        CommandId::ViewSource,
        CommandId::MoreActions,
        CommandId::NextInConversation,
        CommandId::PrevInConversation,
    ] {
        assert_eq!(controller.route(id), Route::Controller, "{id:?}");
    }
    for id in [
        CommandId::Archive,
        CommandId::CaptureTask,
        CommandId::DigestRule,
    ] {
        assert_eq!(controller.route(id), Route::List, "{id:?}");
    }
}

#[test]
fn the_key_map_and_a_dialog_close_on_their_keys_and_give_the_rest_to_the_toolkit() {
    for kind in [SurfaceKind::KeyMap, SurfaceKind::Dialog] {
        let controller = with(kind);
        assert_eq!(controller.route(CommandId::Back), Route::Controller);
        assert_eq!(controller.route(CommandId::CheatSheet), Route::Controller);
        for id in [CommandId::Archive, CommandId::OpenMessage, CommandId::Quit] {
            assert_eq!(controller.route(id), Route::Toolkit, "{kind:?} {id:?}");
        }
    }
}

#[test]
fn back_closes_the_key_map_and_takes_it_off_the_stack() {
    let mut controller = with(SurfaceKind::KeyMap);
    let effects = controller.handle(Input::Command(CommandId::Back));
    assert!(
        shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::KeyMap)),
        "{effects:?}"
    );
    assert_eq!(controller.top_surface(), None);
}

#[test]
fn settings_closes_on_back_and_on_its_own_key() {
    for id in [CommandId::Back, CommandId::Settings] {
        let mut controller = with(SurfaceKind::Settings);
        assert_eq!(controller.route(id), Route::Controller, "{id:?}");
        let effects = controller.handle(Input::Command(id));
        assert!(
            shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Settings)),
            "{id:?}: {effects:?}"
        );
    }
    assert_eq!(
        with(SurfaceKind::Settings).route(CommandId::EditConfig),
        Route::Surface(SurfaceKind::Settings)
    );
}

#[test]
fn the_surfaces_a_frontend_draws_itself_keep_their_keys() {
    for kind in [
        SurfaceKind::Digest,
        SurfaceKind::Capture,
        SurfaceKind::Composer,
    ] {
        let controller = with(kind);
        assert_eq!(controller.route(CommandId::Back), Route::Surface(kind));
        assert_eq!(controller.route(CommandId::Archive), Route::Surface(kind));
    }
}

#[test]
fn the_key_context_is_the_top_surface_s_and_comes_back_when_it_closes() {
    let mut controller = with(SurfaceKind::Message);
    controller.handle(Input::SurfaceOpened(SurfaceKind::Capture));
    assert_eq!(controller.key_context(), KeyContext::Capture);
    controller.handle(Input::SurfaceClosed(SurfaceKind::Capture));
    assert_eq!(controller.key_context(), KeyContext::Reader);
    controller.handle(Input::SurfaceClosed(SurfaceKind::Message));
    assert_eq!(controller.key_context(), KeyContext::List);
}
