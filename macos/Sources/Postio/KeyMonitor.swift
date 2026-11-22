import AppKit
import PostioFFI
import PostioAppKit
import PostioKit

/// The window-level `NSEvent` monitor that dispatches every keystroke. Not `.keyboardShortcut`:
/// SwiftUI's modifier cannot express `g g` sequences, a surface-dependent `Esc`, or typing always
/// winning (ADR 0019 Q4).
/// Menu items draw accelerators from `binding(for:)` with no key equivalent, so nothing races this
/// monitor.
@MainActor
final class KeyMonitor {
    private let resolve: (KeyEvent.Reduced, UiContext, Bool) -> KeyOutcomeFfi
    /// The answer decides whether the key is swallowed: this monitor runs ahead of the responder
    /// chain,
    /// so a key taken for a command nothing handled never reaches the view underneath.
    private let run: (String) -> Bool
    private let pending: (String?) -> Void
    private let context: () -> UiContext

    private var monitor: Any?

    init(
        resolve: @escaping (KeyEvent.Reduced, UiContext, Bool) -> KeyOutcomeFfi,
        run: @escaping (String) -> Bool,
        pending: @escaping (String?) -> Void,
        context: @escaping () -> UiContext
    ) {
        self.resolve = resolve
        self.run = run
        self.pending = pending
        self.context = context
    }

    /// A *local* monitor: a global one needs accessibility permission and reads every keystroke on
    /// the machine.
    func start() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            return self.handle(event) ? nil : event
        }
    }

    func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    /// Separated from the monitor closure so the decision can be exercised without a window server.
    func handle(_ event: NSEvent) -> Bool {
        guard let reduced = KeyEvent.reduce(event) else { return false }
        // Mid-composition the key is the input method's (`KeyDisposition.belongsToInputMethod`).
        if KeyDisposition.belongsToInputMethod(reduced, composing: isComposing()) { return false }

        let typing = Self.isTyping()
        if KeyDisposition.belongsToText(reduced, typing: typing) { return false }
        let grid = NSApp.keyWindow?.firstResponder is FilesCollectionView
        if KeyDisposition.belongsToGrid(reduced, gridFocused: grid) { return false }
        let outcome = resolve(reduced, context(), typing)
        var acted = false
        switch outcome {
        case let .command(id):
            pending(nil)
            acted = run(id)
        case let .pending(description):
            pending(description)
        case .unhandled:
            pending(nil)
        }
        // `KeyDisposition` is in `PostioKit` so it can be tested; this file is in the executable
        // target.
        return KeyDisposition.swallows(outcome: outcome, acted: acted)
    }

    private func isComposing() -> Bool {
        guard let responder = NSApp.keyWindow?.firstResponder as? NSTextInputClient else {
            return false
        }
        return responder.hasMarkedText()
    }

    /// The flag that makes typing win; wrong, a search field archives mail on `a`. Read from the
    /// first
    /// responder, not tracked: a cached copy goes stale exactly when it matters.
    /// `NSTextInputClient` rather than view classes: a responder adopts it because it accepts text.
    static func isTyping() -> Bool {
        // The rule is `TypingResponder`'s, in `PostioKit` where a test can reach it. Asking whether
        // the
        // *window* had a field editor refused every bare-character binding once the toolbar search
        // box made one.
        TypingResponder.isTyping(NSApp.keyWindow?.firstResponder)
    }
}
