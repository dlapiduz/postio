import AppKit
import PostioKit

/// Driven by notifications, not a read of `NSApp.keyWindow` at key time: a menu-bar accelerator
/// fires while the menu owns the event stream, and `keyWindow` is nil there.
@MainActor
public final class KeyWindowTracker {
    public private(set) var current: KeyWindow = .main

    /// Several compose windows can be open, so a verb must reach the one with the keyboard, or Send
    /// in one sends another.
    public private(set) var currentDraft: Int64?

    /// `nonisolated(unsafe)`: `deinit` is not actor-isolated, and removing an observer is safe from
    /// any thread.
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
        // Only if it is the one reported: two compose windows trade the keyboard, so "back to the
        // list" is the one in front giving it up.
        guard Self.role(of: window) == current else { return }
        current = .main
        currentDraft = nil
    }

    /// `NSWindow.identifier` rather than a side table: SwiftUI owns the window's lifetime and hands
    /// it back on every redraw.
    public static func tag(_ window: NSWindow, as role: KeyWindow, draft: Int64? = nil) {
        let tag = draft.map { "\(identifier(for: role))#\($0)" } ?? identifier(for: role)
        window.identifier = NSUserInterfaceItemIdentifier(tag)
    }

    static func draft(of window: NSWindow?) -> Int64? {
        guard let raw = window?.identifier?.rawValue,
              let hash = raw.firstIndex(of: "#")
        else { return nil }
        return Int64(raw[raw.index(after: hash)...])
    }

    /// Untagged is `.main`: a panel AppKit put up itself has no registry bindings.
    static func role(of window: NSWindow?) -> KeyWindow {
        guard let raw = window?.identifier?.rawValue else { return .main }
        let name = raw.split(separator: "#", maxSplits: 1).first.map(String.init) ?? raw
        return KeyWindow.allCases.first { identifier(for: $0) == name } ?? .main
    }

    /// Stricter than `role(of:)`: Edit › Undo must not act on the inbox from AppKit's open panel.
    public static func isMain(_ window: NSWindow?) -> Bool {
        window?.identifier?.rawValue == identifier(for: .main)
    }

    private static func identifier(for role: KeyWindow) -> String {
        switch role {
        case .main: return "postio.window.main"
        case .compose: return "postio.window.compose"
        case .settings: return "postio.window.settings"
        case .message: return "postio.window.message"
        }
    }
}
