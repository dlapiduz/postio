//! Keystrokes, resolved by the core rather than by the frontend.
//!
//! ADR 0019 Q4: *the core owns the keyboard; Swift owns no keymap.* A frontend
//! reduces its own key event to three things every toolkit can supply — the
//! character the key would type, the key's name when it types none, and the
//! modifiers held — and asks. Everything a keymap decides happens on this side
//! of the boundary, which is what keeps `[keys]` meaning the same thing on
//! both platforms.
//!
//! These assert the seam from the *frontend's* side: what a reduced event
//! produces, not what `postio_ui::keymap` does with a chord it was handed.
//! `postio-ui`'s own suite covers the second, and covering only that is how a
//! resolver stays green while nothing reaches it.

use postio_ffi::{KeyOutcomeFfi, ModifiersFfi, Session, SessionOptions, UiContext};

fn session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

/// No modifiers held, which is most presses.
const NONE: ModifiersFfi = ModifiersFfi {
    control: false,
    option: false,
    shift: false,
    command: false,
};

/// The primary accelerator, as this platform's keyboard delivers it.
///
/// `mod` in `[keys]`, expanded when the keymap is built: ⌘ on Apple and
/// Control on freedesktop. One definition, because writing `command: true`
/// inline is a test that passes on a Mac and fails on the runner -- which is
/// exactly what it did, and exactly the mistake
/// `docs/notes/2026-09-05-the-gate-that-runs-cannot-see-the-platform-that-does-not.md`
/// is about, made inside the change that added the note.
const PRIMARY: ModifiersFfi = ModifiersFfi {
    control: !cfg!(target_os = "macos"),
    option: false,
    shift: false,
    command: cfg!(target_os = "macos"),
};

/// A press of a key that types `character`.
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
    // The registry's own defaults, not a keymap this test built: the point of
    // the boundary is that the frontend gets *these*.
    assert_eq!(
        typed(&session, "a", UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "archive".to_string()
        },
        "`a` in the list is archive"
    );
    assert_eq!(
        typed(&session, "u", UiContext::List, false),
        KeyOutcomeFfi::Command {
            id: "undo".to_string()
        }
    );
    session.shutdown();
}

#[test]
fn an_unbound_key_is_left_for_the_frontend_to_pass_on() {
    let session = session();
    // Not "do nothing": the caller has to be able to tell "Postio handled it"
    // from "nobody wanted it", because only the second may reach AppKit.
    assert_eq!(
        typed(&session, "\u{00a7}", UiContext::List, false),
        KeyOutcomeFfi::Unhandled
    );
    session.shutdown();
}

#[test]
fn a_sequence_is_pending_until_its_last_chord() {
    let session = session();
    // `g` alone must not fire whatever `g g` is bound to, and it must not be
    // reported as unhandled either -- the frontend swallows a pending chord so
    // its first key does not also reach the widget underneath, and shows the
    // description so a half-typed sequence is never invisible.
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
    // The rule that decides whether the application feels broken: a search
    // field with focus takes `a`, and the list must not archive on it.
    assert_eq!(
        typed(&session, "a", UiContext::Search, true),
        KeyOutcomeFfi::Unhandled,
        "`a` archived mail while somebody was typing the word"
    );
    // ...and a chord carrying a modifier still fires, or a text field would
    // swallow every shortcut on the machine.
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
    // `mod+k` in `[keys]`, ⌘K on a Mac and Ctrl+K on Linux -- resolved when
    // the keymap is built, so this asserts the *host's* answer and reads the
    // same on both. What it guards is that the expansion and the resolver
    // agree, which they did not: every `mod+…` default was unparseable on
    // Apple until #656.
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
    // The other half of the reduction: a key that types nothing useful sends
    // its name. AppKit spells Escape as U+001B and GDK as `\u{1b}` plus
    // "Escape"; both frontends send the name, and the resolver has one table.
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
    // A dead key mid-composition. It must propagate, or every non-Latin
    // keyboard loses its composition to a monitor that swallowed it.
    assert_eq!(
        session.key(None, None, NONE, UiContext::List, false),
        KeyOutcomeFfi::Unhandled
    );
    session.shutdown();
}
