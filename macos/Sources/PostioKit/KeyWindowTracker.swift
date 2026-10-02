import AppKit

/// Which of Postio's windows has the keyboard, kept current.
///
/// [`KeyboardContext`] decides what a key means given the answer; this is
/// what supplies it. Separated because the decision is a pure function worth
/// asserting on its own, and because this half — the part that listens to
/// AppKit — is exactly the half that has been missing three times in this
/// port. A rule nothing feeds is a rule nothing follows.
///
/// # Notifications, not polling
///
/// `NSApp.keyWindow` read at the moment a key arrives would be simpler and
/// would be wrong in one case that matters: a menu-bar accelerator fires
/// while the menu owns the event stream, and `keyWindow` is nil there. What
/// a key should mean is "the window that last had the keyboard", which is
/// what the notifications say and a snapshot does not.
@MainActor
public final class KeyWindowTracker {
    /// The role of the window that last became key.
    ///
    /// `.main` until something else says otherwise, and back to `.main` when
    /// the window that was in front resigns — closing a compose window puts
    /// the keyboard back where the panes are.
    public private(set) var current: KeyWindow = .main

    /// Which draft the compose window in front is writing, if one is.
    ///
    /// `nil` whenever `current` is not `.compose`. Several compose windows
    /// can be open at once, so "the composer" is not a thing on its own —
    /// a verb has to reach *the one with the keyboard*, or Send in one window
    /// sends another.
    public private(set) var currentDraft: Int64?

    /// The tokens, so the tracker can stop listening.
    ///
    /// `nonisolated(unsafe)` because `deinit` is not actor-isolated and these
    /// are opaque tokens nothing reads — removing an observer is the one
    /// thing they are for, and it is safe from any thread.
    private nonisolated(unsafe) var observers: [NSObjectProtocol] = []
    private nonisolated(unsafe) let center: NotificationCenter

    public init(center: NotificationCenter = .default) {
        self.center = center
        observers = [
            center.addObserver(
                forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main
            ) { [weak self] note in
                guard let window = note.object as? NSWindow else { return }
                MainActor.assumeIsolated { self?.became(window) }
            },
            center.addObserver(
                forName: NSWindow.didResignKeyNotification, object: nil, queue: .main
            ) { [weak self] note in
                guard let window = note.object as? NSWindow else { return }
                MainActor.assumeIsolated { self?.resigned(window) }
            },
        ]
    }

    deinit {
        for observer in observers { center.removeObserver(observer) }
    }

    private func became(_ window: NSWindow) {
        current = Self.role(of: window)
        currentDraft = Self.draft(of: window)
    }

    private func resigned(_ window: NSWindow) {
        // Only if it is the one we are reporting. Two compose windows trade
        // the keyboard between them and neither hand-off means "back to the
        // list"; what does is the one in front giving it up.
        guard Self.role(of: window) == current else { return }
        current = .main
        currentDraft = nil
    }

    /// Mark `window` so the tracker can tell what it is.
    ///
    /// `NSWindow.identifier` rather than a side table: SwiftUI owns the
    /// window's lifetime and hands it back on every redraw, so anything
    /// keyed on object identity has to be kept in step with a thing that is
    /// not ours to watch. The identifier travels with the window.
    public static func tag(_ window: NSWindow, as role: KeyWindow, draft: Int64? = nil) {
        // The draft rides on the identifier rather than in a side table, for
        // the same reason the role does: SwiftUI owns the window's lifetime
        // and hands it back on every redraw, so anything keyed on object
        // identity has to be kept in step with something that is not ours to
        // watch.
        let tag = draft.map { "\(identifier(for: role))#\($0)" } ?? identifier(for: role)
        window.identifier = NSUserInterfaceItemIdentifier(tag)
    }

    /// Which draft `window` is writing, if its tag says.
    static func draft(of window: NSWindow?) -> Int64? {
        guard let raw = window?.identifier?.rawValue,
              let hash = raw.firstIndex(of: "#")
        else { return nil }
        return Int64(raw[raw.index(after: hash)...])
    }

    /// What an untagged window counts as, and what each tag means.
    ///
    /// Untagged is `.main`: a panel AppKit put up itself — an open dialog, a
    /// font panel — has no registry bindings of its own, and answering
    /// `.main` keeps exactly the behaviour there was before any of this.
    static func role(of window: NSWindow?) -> KeyWindow {
        guard let raw = window?.identifier?.rawValue else { return .main }
        // The tag may carry a draft after a `#`; the role is the part before.
        let name = raw.split(separator: "#", maxSplits: 1).first.map(String.init) ?? raw
        return KeyWindow.allCases.first { identifier(for: $0) == name } ?? .main
    }

    private static func identifier(for role: KeyWindow) -> String {
        switch role {
        case .main: return "postio.window.main"
        case .compose: return "postio.window.compose"
        case .settings: return "postio.window.settings"
        }
    }
}
