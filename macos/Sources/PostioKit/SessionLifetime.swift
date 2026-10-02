/// The application's phase, as far as the session's lifetime cares.
///
/// Not SwiftUI's `ScenePhase`: that type lives in the executable target,
/// which nothing can test, and it has no case for "quitting" — the moment
/// that actually matters here. Three of these map from `ScenePhase` and the
/// fourth comes from `applicationWillTerminate`.
public enum SessionPhase: CaseIterable, Sendable {
    /// A window is in front.
    case active
    /// A window exists and is not in front.
    case inactive
    /// No window is showing: `⌘W`, `⌘H`, or minimised.
    case background
    /// The application is quitting.
    case terminating
}

/// When Postio is allowed to stop collecting mail.
///
/// # The bug this is
///
/// The session was ended on `ScenePhase.background` — which on macOS is what
/// `⌘W`, `⌘H` and minimising all produce. So the gesture a Mac user makes to
/// *leave a mail client running* was the gesture that stopped every sync,
/// every IDLE connection and every new-mail notification. And it did not come
/// back: the session is opened once, in `Engine.init`, so reopening a window
/// found the engine `.open` with nothing behind it — a full mailbox drawing
/// "No messages", no keyboard, and no recovery short of quitting.
///
/// GTK never did this. `postio-app` stops the runtime after `run()` returns,
/// which is process exit, and nothing there stops syncing because a window
/// stopped being visible.
///
/// # Why it was written that way, and why that reason is gone
///
/// The comment said: *"the store is SQLCipher, and dropping an engine as the
/// process ends is exactly when libcrypto goes away underneath a thread still
/// encrypting a page."* That was true of the store Postio had. The engine is
/// Turso now and there is no libcrypto to go away — so the orderly shutdown
/// is still worth having, and `terminating` is where it belongs.
public enum SessionLifetime {
    /// Whether entering `phase` should end the session.
    public static func shouldEnd(on phase: SessionPhase) -> Bool {
        switch phase {
        case .terminating:
            // Orderly rather than by dropping it as the process unwinds.
            return true
        case .active, .inactive, .background:
            // A mail client with no window on screen is a mail client
            // collecting mail. That is what it is *for*.
            return false
        }
    }
}
