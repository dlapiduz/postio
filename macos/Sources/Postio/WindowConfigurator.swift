import AppKit
import PostioAppKit
import PostioKit
import SwiftUI

/// Reaches the `NSWindow` behind a SwiftUI scene, once: SwiftUI has no modifier for frame autosave
/// or for putting a window back on a screen that still exists.
struct WindowConfigurator: NSViewRepresentable {
    /// Shared by every window, so `NSWindow` restores the frame rather than cascading a new one.
    static let autosaveName = "PostioMainWindow"

    /// Tagged here, the one place that reaches the `NSWindow`: what a key means depends on which
    /// window it was typed into, and SwiftUI does not say.
    var role: KeyWindow = .main
    /// So a verb reaches the compose window with the keyboard; several may be open.
    var draft: Int64?

    func makeNSView(context _: Context) -> NSView {
        let view = NSView(frame: .zero)
        // `window` is nil until the view joins a hierarchy: one hop, not a poll.
        let role = role
        let draft = draft
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            KeyWindowTracker.tag(window, as: role, draft: draft)
            // Postio's surface rather than AppKit's, so the titlebar and anything a pane does not
            // cover are one ramp (#1588).
            window.backgroundColor = AppSurface.background
            if role == .settings, DemoMode.seed != nil {
                window.setContentSize(CGSize(width: 900, height: 760))
            }
            guard role == .main else { return }
            hideTitle(of: window)
            if DemoMode.seed != nil {
                window.setContentSize(DemoMode.windowSize)
                window.center()
                return
            }
            window.setFrameAutosaveName(Self.autosaveName)
            recover(window)
        }
        return view
    }

    func updateNSView(_: NSView, context _: Context) {}

    /// SwiftUI titles a `WindowGroup` from its scene and puts the title back on every scene update,
    /// so it is hidden and emptied and re-applied whenever the window updates.
    @MainActor
    private func hideTitle(of window: NSWindow) {
        Self.emptyTitle(of: window)
        NotificationCenter.default.addObserver(
            forName: NSWindow.didUpdateNotification,
            object: window,
            queue: .main
        ) { note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated { Self.emptyTitle(of: window) }
        }
    }

    @MainActor
    private static func emptyTitle(of window: NSWindow) {
        guard window.titleVisibility != .hidden || !window.title.isEmpty else { return }
        window.titleVisibility = .hidden
        window.title = ""
    }

    /// `NSWindow`'s autosave does not cover a frame saved on an unplugged display, which reopens
    /// where nothing can reach.
    private func recover(_ window: NSWindow) {
        let screens = NSScreen.screens.map(\.visibleFrame)
        let corrected = WindowState.visibleFrame(for: window.frame, on: screens)
        guard corrected != window.frame else { return }
        window.setFrame(corrected, display: true)
    }
}
