import AppKit
import PostioFFI
import PostioKit

/// The window-level `NSEvent` monitor that dispatches every keystroke.
///
/// **Not `.keyboardShortcut`** (ADR 0019 Q4). SwiftUI's modifier is a
/// menu-accelerator model, and it cannot express any of the three things this
/// application's keyboard is built on: a `g g` sequence, an `Esc` whose
/// meaning depends on which surface has focus, or *typing always wins*. A
/// local monitor can, and it is the shape the resolver expects. Menu items
/// draw their accelerators from `binding(for:)` and are given **no key
/// equivalent**, so nothing races this for the same keystroke.
///
/// It owns no keymap. Everything it does is: reduce the event, ask, and act on
/// one of three answers.
@MainActor
final class KeyMonitor {
    /// Ask the boundary what a press means.
    private let resolve: (KeyEvent.Reduced, UiContext, Bool) -> KeyOutcomeFfi
    /// Run a command the boundary named.
    private let run: (String) -> Void
    /// Show, or clear, a half-typed sequence.
    private let pending: (String?) -> Void
    /// Which surface has focus, as the application understands it.
    private let context: () -> UiContext

    private var monitor: Any?

    init(
        resolve: @escaping (KeyEvent.Reduced, UiContext, Bool) -> KeyOutcomeFfi,
        run: @escaping (String) -> Void,
        pending: @escaping (String?) -> Void,
        context: @escaping () -> UiContext
    ) {
        self.resolve = resolve
        self.run = run
        self.pending = pending
        self.context = context
    }

    /// Start listening, until `stop()`.
    ///
    /// A *local* monitor: it sees this application's events only. A global one
    /// would need accessibility permission and would read every keystroke on
    /// the machine, which is not something a mail client gets to ask for.
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

    /// Whether Postio handled `event`, and it should therefore be swallowed.
    ///
    /// Separated from the monitor closure so the decision can be exercised
    /// without a window server: what this returns is exactly what decides
    /// whether a key reaches the widget underneath.
    func handle(_ event: NSEvent) -> Bool {
        // Mid-composition, always. An IME builds a character over several
        // key presses and marks the text while it does; swallowing one of
        // those would break every keyboard that composes, and no binding is
        // worth that. Asked of the responder rather than the event, because
        // it is a fact about what the input context is holding.
        if isComposing() { return false }
        guard let reduced = KeyEvent.reduce(event) else { return false }

        let typing = Self.isTyping()
        switch resolve(reduced, context(), typing) {
        case let .command(id):
            pending(nil)
            run(id)
            return true
        case let .pending(description):
            // Swallowed, so the first chord of a sequence does not also reach
            // the widget underneath, and shown, so a half-typed `g` is never
            // invisible.
            pending(description)
            return true
        case .unhandled:
            pending(nil)
            return false
        }
    }

    /// Whether an input method is part-way through composing a character.
    private func isComposing() -> Bool {
        guard let responder = NSApp.keyWindow?.firstResponder as? NSTextInputClient else {
            return false
        }
        return responder.hasMarkedText()
    }

    /// Whether the focused responder takes text.
    ///
    /// The flag the resolver uses to make typing win, and the most visible
    /// thing this file can get wrong: a search field that archives mail on
    /// `a` reads as a broken application, not as a misrouted key. Answered
    /// from the first responder rather than tracked, because focus is a live
    /// property and a cached copy goes stale in exactly the window that
    /// matters.
    ///
    /// `NSTextInputClient` rather than a list of view classes: it is the
    /// protocol a responder adopts *because* it accepts text, so a field this
    /// application has not thought of is covered by construction.
    static func isTyping() -> Bool {
        guard let responder = NSApp.keyWindow?.firstResponder else { return false }
        if responder is NSTextInputClient { return true }
        // An `NSTextField` delegates its editing to a shared field editor, so
        // the first responder while somebody types into one is the editor,
        // not the field. Both are covered above; this is the case where the
        // field itself is focused and the editor has not been installed yet.
        return (responder as? NSView)?.window?.fieldEditor(false, for: responder) != nil
    }
}
