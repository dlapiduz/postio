import PostioFFI

/// `KeyMonitor` runs ahead of the responder chain, and what it swallows never reaches AppKit.
/// Swallowing every key the resolver claimed
/// made a command with no handler worse than a no-op: `space` resolved to `scroll_reader_down`, was
/// swallowed, and never reached the
/// scroll view that would have paged it. Claiming a key and acting on it are two things; only
/// acting earns the swallow.
public enum KeyDisposition {
    public static func swallows(outcome: KeyOutcomeFfi, acted: Bool) -> Bool {
        switch outcome {
        case .command:
            return acted
        case .pending:
            // Always: the first chord of a sequence must not also reach the widget underneath (`g`
            // would type a `g`).
            return true
        case .unhandled:
            return false
        }
    }

    /// Every key but a Command chord: an input method builds a character over several presses, and
    /// swallowing one would break every
    /// keyboard that composes. AppKit hands a Command chord to the menus first, and marked text
    /// also holds inline predictive text,
    /// which left ⌘↩ unsent from the compose body.
    public static func belongsToInputMethod(_ key: KeyEvent.Reduced, composing: Bool) -> Bool {
        composing && !key.modifiers.command
    }

    /// The resolver lets typing win over bare characters only, so ⌘A in a text field resolved to
    /// "select all messages" and the field
    /// never selected its text. These chords are the platform's in a field whatever `[keys]` says;
    /// other chords (⌘K) still reach their command.
    public static func belongsToText(_ key: KeyEvent.Reduced, typing: Bool) -> Bool {
        guard typing, key.modifiers.command, !key.modifiers.control, !key.modifiers.option,
              let character = key.character?.lowercased()
        else { return false }
        return ["a", "c", "v", "x", "z"].contains(character)
    }

    /// ← → move a card and ↑ ↓ a row of four, as any collection view's do; the resolver would read
    /// ↓ as the list's next and → as open. A chord still resolves.
    public static func belongsToGrid(_ key: KeyEvent.Reduced, gridFocused: Bool) -> Bool {
        guard gridFocused, let name = key.name else { return false }
        let modifiers = key.modifiers
        guard !modifiers.command, !modifiers.control, !modifiers.option, !modifiers.shift else { return false }
        return ["left", "right", "up", "down"].contains(name.lowercased())
    }
}
