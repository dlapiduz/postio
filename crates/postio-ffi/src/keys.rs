//! Keystrokes, resolved by the core: Swift owns no keymap (ADR 0019 Q4). A frontend reduces its key
//! event to the character,
//! the key's name and the modifiers held, and `postio_ui::keymap` owns bindings, chords, sequences
//! and the leader timeout.

/// The three answers of `postio_ui::keymap::Outcome`; only `Unhandled` lets the key propagate to
/// AppKit.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum KeyOutcomeFfi {
    Command {
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

/// A record of four booleans, not a bitmask, so the meaning of each bit does not live in comments
/// on both sides.
/// `command` is ⌘ and maps to `Super`, which `mod` expands to on this platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ModifiersFfi {
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
