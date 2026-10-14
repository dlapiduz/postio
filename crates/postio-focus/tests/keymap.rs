//! `[keys]` reloaded, and the key map, from the controller's side
//! (specs/009-focus-macos T102; screen 20).
//!
//! What GTK's window did in `set_keymap` and its CheatSheet toggle, asserted
//! here so both frontends get it: a new keymap is what the controller
//! resolves and what every key it spells names, at once and without a
//! restart; `?` opens the key map and closes it again, and while it is up
//! the keyboard is its.

use std::time::Instant;

use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::{CommandId, ConnectionState, Event, Keymap};
use postio_focus::{Effect, FocusController, Input, Intent, Policy, SurfaceKind};
use postio_ui::keymap::{Chord, KeyContext, Outcome};

fn focus() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

fn press(focus: &mut FocusController, key: &str) -> Outcome {
    let chord: Chord = key.parse().expect("a chord");
    focus.press(&chord, KeyContext::List, false, Instant::now())
}

/// The registry's keys, with `command` bound to `key` instead.
fn rebound(command: &str, key: &str) -> Keymap {
    let mut keys = KeyBindings::default();
    keys.overrides_mut()
        .insert(command.to_owned(), key.to_owned());
    let keymap = Keymap::resolve_on(&keys, Platform::Apple);
    assert!(
        keymap.problems().is_empty(),
        "the rebinding is a clean one: {:?}",
        keymap.problems()
    );
    keymap
}

#[test]
fn a_new_keymap_is_what_the_controller_resolves() {
    let mut focus = focus();
    assert_eq!(
        press(&mut focus, "a"),
        Outcome::Command("archive".to_owned())
    );
    let _ = focus.handle(Input::Keymap(rebound("archive", "F9")));
    assert_eq!(
        press(&mut focus, "F9"),
        Outcome::Command("archive".to_owned()),
        "the new binding resolves"
    );
    assert_ne!(
        press(&mut focus, "a"),
        Outcome::Command("archive".to_owned()),
        "the old one no longer archives"
    );
}

#[test]
fn a_new_keymap_is_what_every_key_the_controller_spells_says() {
    let mut focus = focus();
    let _ = focus.handle(Input::Event(Event::ConnectionChanged {
        account: 1.into(),
        state: ConnectionState::Offline,
    }));
    let effects = focus.handle(Input::Keymap(rebound("refresh", "F9")));
    assert_eq!(
        focus
            .keymap()
            .bindings(CommandId::Refresh)
            .first()
            .map(String::as_str),
        Some("F9")
    );
    let keys: Vec<Option<String>> = shown(&effects)
        .into_iter()
        .filter_map(|intent| match intent {
            Intent::Banner(Some(banner)) => banner.button.map(|button| button.key),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        vec![Some("F9".to_owned())],
        "the offline banner's Retry names its new key, without a restart"
    );
}

#[test]
fn question_mark_opens_the_key_map_and_closes_it() {
    let mut focus = focus();
    assert!(
        focus.answers(CommandId::CheatSheet),
        "`?` is the controller's"
    );
    assert_eq!(
        shown(&focus.handle(Input::Command(CommandId::CheatSheet))),
        vec![Intent::OpenKeyMap]
    );
    assert!(focus.has_surface(), "the key map is on the stack at once");
    assert_eq!(
        shown(&focus.handle(Input::Command(CommandId::CheatSheet))),
        vec![Intent::CloseSurface(SurfaceKind::KeyMap)],
        "`?` again closes it"
    );
    assert_eq!(
        shown(&focus.handle(Input::SurfaceClosed(SurfaceKind::KeyMap))),
        vec![Intent::KeyboardHome]
    );
    assert!(!focus.has_surface());
}

#[test]
fn the_key_map_opens_over_a_window_and_leaves_it_open() {
    let mut focus = focus();
    let _ = focus.handle(Input::SurfaceOpened(SurfaceKind::Digest));
    assert_eq!(
        shown(&focus.handle(Input::Command(CommandId::CheatSheet))),
        vec![Intent::OpenKeyMap],
        "the key map is not a window: the digest stays"
    );
    assert_eq!(
        shown(&focus.handle(Input::Command(CommandId::Back))),
        vec![Intent::CloseSurface(SurfaceKind::KeyMap)]
    );
    let _ = focus.handle(Input::SurfaceClosed(SurfaceKind::KeyMap));
    assert_eq!(focus.key_context(), KeyContext::Digest);
}

#[test]
fn while_the_key_map_is_up_the_keyboard_is_its() {
    let mut focus = focus();
    let _ = focus.handle(Input::Command(CommandId::CheatSheet));
    for key in [
        CommandId::NextMessage,
        CommandId::Archive,
        CommandId::Search,
    ] {
        assert!(
            focus.answers(key),
            "{key:?} is swallowed, not sent on to the host"
        );
        assert!(
            shown(&focus.handle(Input::Command(key))).is_empty(),
            "{key:?} does nothing under the key map"
        );
    }
    assert_eq!(
        shown(&focus.handle(Input::Command(CommandId::Quit))),
        vec![Intent::Quit],
        "Quit still quits"
    );
}

#[test]
fn the_bar_up_is_drawn_again_under_the_new_keys() {
    // GTK handed the new keymap to the bar and every picker: what is on
    // screen spells its keys anew, not at the next keystroke.
    let mut focus = focus();
    let _ = focus.handle(Input::Command(CommandId::Search));
    let effects = focus.handle(Input::Keymap(rebound("archive", "F9")));
    assert!(
        shown(&effects)
            .iter()
            // On the Mac the bar's search half is the dropdown (spec 010).
            .any(|intent| matches!(intent, Intent::BarLines(_) | Intent::Dropdown(_))),
        "the bar's lines are drawn again: {effects:?}"
    );
}
