import AppKit
import PostioKit

/// Edit › Undo over the engine's undo stack (specs/009-focus-macos T051,
/// R7, FR-041).
///
/// The engine's `UndoStack` is the only stack. This holds nothing of its
/// own but the words for the stack's top, read from the engine
/// (`undoDescription`), and sends `undo` when chosen: the same command `u`,
/// `⌘Z` and the toast's Undo run. Mirroring each action with
/// `registerUndo(withTarget:)` was rejected in R7 -- the mirror drifts the
/// moment the toast's Undo, an expiry or another window changes the stack.
///
/// The words are cached because AppKit asks for them synchronously, while
/// validating the menu, and the engine's answer is a store read that may
/// not be made on the main actor. They are refreshed on each toast (every
/// verb and every undo says one) and when the main window becomes key.
///
/// There is no redo: the engine has none.
@MainActor
public final class PostioUndoManager: UndoManager {
    /// Sends the engine's `undo`.
    private let sendUndo: () -> Void

    /// What Undo would take back, in the toast's words, or `nil`.
    public private(set) var described: String?

    /// Which refresh is the newest, so an older read that lands late never
    /// overwrites a newer one.
    private var reading = 0

    /// `undo` is what choosing the item does: `session.invoke("undo")`.
    public init(undo: @escaping () -> Void) {
        sendUndo = undo
        super.init()
        // Nothing is ever registered here; grouping by event would only
        // open groups that nothing closes.
        groupsByEvent = false
    }

    /// Hold `description` as the stack's top.
    public func remember(_ description: String?) {
        reading += 1
        described = description
    }

    /// Read the stack's top with `read`, off the main actor, and hold it.
    public func refresh(_ read: @escaping @Sendable () -> String?) async {
        reading += 1
        let mine = reading
        let found = await Task.detached(priority: .userInitiated) { read() }.value
        guard mine == reading else { return }
        described = found
    }

    public override var canUndo: Bool { described != nil }
    public override var canRedo: Bool { false }
    public override var undoActionName: String { described ?? "" }
    public override var redoActionName: String { "" }

    public override var undoMenuItemTitle: String {
        described.map { "Undo \($0)" } ?? "Undo"
    }

    public override var redoMenuItemTitle: String { "Redo" }

    public override func undoMenuTitle(forUndoActionName actionName: String) -> String {
        actionName.isEmpty ? "Undo" : "Undo \(actionName)"
    }

    public override func undo() {
        sendUndo()
    }

    public override func redo() {}
}

/// Where Edit › Undo and Redo go (T051).
///
/// **Not `windowWillReturnUndoManager`, as R7 planned, and measured why.**
/// SwiftUI's window asks for its undo manager while it is being built --
/// before any view of ours can reach the window -- and an `NSWindow` that
/// has made its own manager never asks its delegate again. A delegate
/// installed afterwards (a forwarding proxy over SwiftUI's
/// `AppKitWindowController`) was never consulted; the probe is in the
/// commit that added this.
///
/// So the menu's Undo item is aimed here. In the main window, while
/// nothing there takes text, it is the engine's (`PostioUndoManager`):
/// titled with the stack's top, enabled while there is one. Anywhere else
/// -- a text field, a compose window, Settings -- it goes on down the
/// responder chain exactly as AppKit's own `undo:` would, so typing still
/// undoes typing with the field editor's own manager.
@MainActor
public final class UndoRouter: NSObject, NSMenuItemValidation {
    public let manager: PostioUndoManager

    /// The window the menu acts on: the key window, or the main window
    /// while a menu has the event stream.
    var window: () -> NSWindow? = { NSApp.keyWindow ?? NSApp.mainWindow }

    public init(manager: PostioUndoManager) {
        self.manager = manager
    }

    /// The engine's manager, if `window` is Postio's main window and its
    /// first responder does not take text.
    func engineUndo(in window: NSWindow?, firstResponder: NSResponder? = nil) -> PostioUndoManager? {
        guard let window, KeyWindowTracker.isMain(window) else { return nil }
        let responder = firstResponder ?? window.firstResponder
        return TypingResponder.isTyping(responder) ? nil : manager
    }

    @objc public func undo(_ sender: Any?) {
        if let engine = engineUndo(in: window()) {
            engine.undo()
        } else {
            NSApp.sendAction(Self.undoAction, to: nil, from: sender)
        }
    }

    @objc public func redo(_ sender: Any?) {
        if engineUndo(in: window()) != nil { return }
        NSApp.sendAction(Self.redoAction, to: nil, from: sender)
    }

    public func validateMenuItem(_ item: NSMenuItem) -> Bool {
        let undoing = item.action == Self.undoAction
        if let engine = engineUndo(in: window()) {
            item.title = undoing ? engine.undoMenuItemTitle : engine.redoMenuItemTitle
            return undoing ? engine.canUndo : engine.canRedo
        }
        // AppKit's own: whoever answers `undo:` down the chain titles and
        // enables the item, as it would with no router in the way.
        item.title = undoing ? "Undo" : "Redo"
        guard let action = item.action,
              let target = NSApp.target(forAction: action, to: nil, from: item) as? NSObject
        else { return false }
        if let validating = target as? NSMenuItemValidation {
            return validating.validateMenuItem(item)
        }
        if let validating = target as? NSUserInterfaceValidations {
            return validating.validateUserInterfaceItem(item)
        }
        return true
    }

    /// `undo:` and `redo:`: `NSResponder`'s by convention, with no
    /// declaration of AppKit's to take a `#selector` of, so this type's own
    /// are taken -- the same selectors, which is what lets the router hand
    /// them on down the chain unchanged.
    public static let undoAction = #selector(UndoRouter.undo(_:))
    public static let redoAction = #selector(UndoRouter.redo(_:))
}
