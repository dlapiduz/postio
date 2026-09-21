import AppKit
import PostioKit
import SwiftUI

/// Reaches the `NSWindow` behind a SwiftUI scene, once, to set up autosave
/// and to say which window it is.
///
/// SwiftUI has no modifier for frame autosave and none for "put this back on a
/// screen that still exists", so this is the standard escape hatch: a
/// zero-size representable that reads `view.window` after the view is in a
/// hierarchy.
struct WindowConfigurator: NSViewRepresentable {
    /// Shared by every Postio window, which is what makes `NSWindow` restore
    /// the frame rather than cascade a new one.
    static let autosaveName = "PostioMainWindow"

    /// Which window this is, for `KeyWindowTracker`.
    ///
    /// The key monitor sees every key press in the application, so what a key
    /// means depends on which window it was typed into — and SwiftUI does not
    /// say. Tagging is how the tracker can tell, and it happens here because
    /// this is already the one place that reaches the `NSWindow`.
    var role: KeyWindow = .main
    /// Which draft a compose window is writing, so a verb can reach the one
    /// with the keyboard rather than "the composer", which is not a thing
    /// when several are open.
    var draft: Int64?

    func makeNSView(context _: Context) -> NSView {
        let view = NSView(frame: .zero)
        // `window` is nil until the view joins a hierarchy, which is after
        // this returns. One hop, not a poll.
        let role = role
        let draft = draft
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            KeyWindowTracker.tag(window, as: role, draft: draft)
            // Postio's own surface, on every window, rather than AppKit's.
            // The panes paint over most of it; what this settles is the
            // titlebar and anything a pane does not cover, so the window is
            // one ramp edge to edge. See `AppSurface` (#1588).
            window.backgroundColor = AppSurface.background
            // Only the main window's frame is worth restoring, and only it
            // wants an empty title bar: a compose window says who it is
            // writing to, and the settings window says "Settings".
            guard role == .main else { return }
            window.setFrameAutosaveName(Self.autosaveName)
            hideTitle(of: window)
            recover(window)
            persistSplits(in: window)
        }
        return view
    }

    func updateNSView(_: NSView, context _: Context) {}

    /// Keep the title bar empty, the way the canvas draws it.
    ///
    /// The application's own name in its own chrome is a line of the window
    /// spent telling you what you knew when you opened it. SwiftUI titles a
    /// `WindowGroup` from its scene and puts it back whenever the scene
    /// updates — the same habit that took the menu bar (#1262) — so setting
    /// it once at launch lasted until the first redraw. Hidden *and* emptied,
    /// and re-applied whenever the window updates: `titleVisibility` alone
    /// left the string to come back the moment something restored it.
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

    /// Give the split view an autosave name so its column widths persist.
    ///
    /// `NavigationSplitView` is an `NSSplitViewController` underneath and has
    /// no SwiftUI modifier for this. Naming it is all AppKit needs; without it
    /// the panes return to their ideal widths on every launch, which is one of
    /// the things that makes an application read as a prototype.
    ///
    /// Best-effort by design: a future SwiftUI that is not backed by
    /// `NSSplitView` finds nothing here and the app keeps working with its
    /// default widths, which is the right way for a reach into someone else's
    /// view hierarchy to fail.
    private func persistSplits(in window: NSWindow) {
        guard let root = window.contentView,
              let split = firstSplitView(in: root)
        else { return }
        split.autosaveName = "PostioPanes"
    }

    private func firstSplitView(in view: NSView) -> NSSplitView? {
        if let split = view as? NSSplitView { return split }
        for child in view.subviews {
            if let found = firstSplitView(in: child) { return found }
        }
        return nil
    }

    /// Bring a window back onto a screen that still exists.
    ///
    /// The case `NSWindow`'s own autosave does not cover: a frame saved on a
    /// second display that has since been unplugged reopens somewhere nothing
    /// can reach, and Postio looks like it failed to launch.
    private func recover(_ window: NSWindow) {
        let screens = NSScreen.screens.map(\.visibleFrame)
        let corrected = WindowState.visibleFrame(for: window.frame, on: screens)
        guard corrected != window.frame else { return }
        window.setFrame(corrected, display: true)
    }
}
