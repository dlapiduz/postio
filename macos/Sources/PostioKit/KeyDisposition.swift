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
}
