//! Keystrokes, resolved by the core.
//!
//! **Swift owns no keymap** (ADR 0019 Q4). A frontend reduces its own key
//! event to the three things every toolkit can supply — the character the key
//! would type, the key's name when it types none, and the modifiers held —
//! and asks. `postio_ui::keymap` owns the binding table, the chords, the
//! sequences and the leader timeout, on both platforms, so `[keys]` means the
//! same thing in both and `docs/keybindings.md` describes both.
//!
//! The alternative was SwiftUI's `.keyboardShortcut`, and it cannot express
//! any of the three things this has to: a `g g` sequence, an `Esc` whose
//! meaning depends on the focused surface, or "typing always wins".

/// What a key press meant.
///
/// The same three answers `postio_ui::keymap::Outcome` gives, named for the
/// boundary. Swift acts on all three: it runs the command, it shows the
/// pending prefix, and — only for [`KeyOutcomeFfi::Unhandled`] — it lets the
/// key propagate to whatever AppKit would have done with it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyOutcomeFfi {
    /// Run this command. The registry's own id, for [`crate::Session::invoke`].
    Command {
        /// The registry id, as `[keys]` and every log line spell it.
        id: String,
    },
    /// The start of a sequence, and what to show while it is half-typed —
    /// `"g"` — so the pending state is never invisible.
    Pending {
        /// The chords typed so far, for the frontend to show.
        description: String,
    },
    /// Nothing is bound to it. The caller should let the key propagate.
    Unhandled,
}

impl From<postio_ui::keymap::Outcome> for KeyOutcomeFfi {
    fn from(outcome: postio_ui::keymap::Outcome) -> Self {
        use postio_ui::keymap::Outcome;
        match outcome {
            Outcome::Command(id) => KeyOutcomeFfi::Command { id },
            Outcome::Pending(description) => KeyOutcomeFfi::Pending { description },
            Outcome::Unhandled => KeyOutcomeFfi::Unhandled,
        }
    }
}

/// The modifiers held with a key, as a platform reports them.
///
/// A record of four booleans rather than a bitmask: uniffi would carry a
/// `u8` happily enough, and then the meaning of each bit would live in a
/// comment on each side of the boundary. `NSEvent.ModifierFlags` and
/// `Modifiers` are both already sets; this is the shape that cannot be
/// assembled wrongly.
///
/// `command` is Apple's ⌘ and maps to `Super`, which is what `mod` expands to
/// on this platform (`postio_config::keys::expand_mod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ModifiersFfi {
    /// ⌃ on Apple, Control everywhere.
    pub control: bool,
    /// ⌥. `Alt` to the keymap, which is the spelling `[keys]` uses.
    pub option: bool,
    /// ⇧. Folded into the character for a key that types one, which
    /// [`postio_ui::keymap::Chord`] does rather than this.
    pub shift: bool,
    /// ⌘, and the same physical modifier X11 calls Super — which is what
    /// `mod` expands to on this platform.
    pub command: bool,
}

impl From<ModifiersFfi> for postio_ui::keymap::Modifiers {
    fn from(held: ModifiersFfi) -> Self {
        use postio_ui::keymap::Modifiers;
        let mut modifiers = Modifiers::NONE;
        if held.control {
            modifiers = modifiers.with(Modifiers::CTRL);
        }
        if held.option {
            modifiers = modifiers.with(Modifiers::ALT);
        }
        if held.shift {
            modifiers = modifiers.with(Modifiers::SHIFT);
        }
        if held.command {
            modifiers = modifiers.with(Modifiers::SUPER);
        }
        modifiers
    }
}
