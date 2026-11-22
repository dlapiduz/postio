/// Not SwiftUI's `ScenePhase`: that lives in the executable target, which nothing can test, and has
/// no "quitting" case. Three map from it; the fourth comes from `applicationWillTerminate`.
public enum SessionPhase: CaseIterable, Sendable {
    case active
    case inactive
    /// `⌘W`, `⌘H`, or minimised.
    case background
    case terminating
}

/// The session used to end on `ScenePhase.background`, which `⌘W`, `⌘H` and minimising all produce:
/// that stopped sync, IDLE and notifications, and reopening a window found an engine `.open` with
/// nothing behind it. A mail client with no window keeps collecting mail; only `terminating` stops
/// it, for an orderly shutdown.
public enum SessionLifetime {
    public static func shouldEnd(on phase: SessionPhase) -> Bool {
        switch phase {
        case .terminating:
            return true
        case .active, .inactive, .background:
            // A mail client with no window on screen keeps collecting mail.
            return false
        }
    }
}
