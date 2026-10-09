import PostioFFI

/// Whether a key press is swallowed or left to the view underneath.
///
/// # The bug this is
///
/// `KeyMonitor` is a *local* `NSEvent` monitor: it runs ahead of the
/// responder chain, and whatever it returns `nil` for never reaches AppKit at
/// all. It swallowed every key the resolver claimed — whether or not anything
/// then acted on it.
///
/// So a command the registry knows and this frontend has not built yet was
/// **worse than a no-op**: `space` in the reading pane resolved to
/// `scroll_reader_down`, was swallowed, reached no handler, and never got to
/// the scroll view that would have paged it natively. A key that does nothing
/// reads as a broken application; a key that does nothing *and* stops the
/// platform's own behaviour reads as a broken application that is trying.
///
/// The rule is that claiming a key and acting on it are two things, and only
/// the second earns the swallow.
public enum KeyDisposition {
    /// Whether to swallow, given what the resolver said and whether the
    /// application actually acted.
    public static func swallows(outcome: KeyOutcomeFfi, acted: Bool) -> Bool {
        switch outcome {
        case .command:
            // Only if something happened. Otherwise the view underneath gets
            // its turn, which for `space` in a scroll view is exactly the
            // behaviour being asked for.
            return acted
        case .pending:
            // Always. The first chord of a sequence must not also reach the
            // widget underneath — `g` in the list would type a `g` into
            // whatever takes text next.
            return true
        case .unhandled:
            return false
        }
    }

    /// Whether `key` is the focused text field's own editing -- select all,
    /// copy, cut, paste, undo, redo -- and so never a command while somebody
    /// is typing.
    ///
    /// The resolver lets typing win over bare characters only; a modifier
    /// chord still resolves. So `⌘A` in a text field resolved to the
    /// registry's "select all messages", was swallowed by the monitor, and
    /// the field never selected its text. These chords are the platform's,
    /// in a field, whatever `[keys]` says; every other chord (`⌘K` for the
    /// palette, say) still reaches its command from a field.
    public static func belongsToText(_ key: KeyEvent.Reduced, typing: Bool) -> Bool {
        guard typing, key.modifiers.command, !key.modifiers.control, !key.modifiers.option,
              let character = key.character?.lowercased()
        else { return false }
        return ["a", "c", "v", "x", "z"].contains(character)
    }

    /// Whether `key` is a bare arrow for the Files tab's grid while it has
    /// the keyboard (spec 010 T130, design §3.8): ← → move a card, ↑ ↓ a
    /// row of four, as any collection view's do, and the grid says where
    /// the ring went. The resolver would read ↓ as the list's next and →
    /// as open, which on a grid are the wrong two things. A chord still
    /// resolves: ⌘↓ saves the file, ⌥← steps the timeline.
    public static func belongsToGrid(_ key: KeyEvent.Reduced, gridFocused: Bool) -> Bool {
        guard gridFocused, let name = key.name else { return false }
        let modifiers = key.modifiers
        guard !modifiers.command, !modifiers.control, !modifiers.option, !modifiers.shift else { return false }
        return ["left", "right", "up", "down"].contains(name.lowercased())
    }
}
