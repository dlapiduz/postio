import AppKit
import PostioKit

/// The engine's `UndoStack` is the only stack; mirroring actions with `registerUndo(withTarget:)`
/// was rejected (R7) because the mirror drifts when the toast's Undo, an expiry or another window
/// changes the stack. The words are cached because AppKit asks synchronously during menu validation
/// and the engine's answer is a store read that may not run on the main actor. There is no redo.
@MainActor
public final class PostioUndoManager: UndoManager {
    private let sendUndo: () -> Void

    public private(set) var described: String?

    /// Newest refresh wins, so an older read landing late never overwrites a newer one.
    private var reading = 0

    public init(undo: @escaping () -> Void) {
        sendUndo = undo
        super.init()
        groupsByEvent = false
    }

    public func remember(_ description: String?) {
        reading += 1
        described = description
    }

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

/// Not `windowWillReturnUndoManager`: SwiftUI's window asks for its undo manager while being built,
/// and an `NSWindow` that made its own never asks its delegate again. In the main window, while
/// nothing takes text, Undo is the engine's; anywhere else it goes down the responder chain as
/// AppKit's `undo:` would, so typing still undoes typing.
@MainActor
public final class UndoRouter: NSObject, NSMenuItemValidation {
    public let manager: PostioUndoManager

    var window: () -> NSWindow? = { NSApp.keyWindow ?? NSApp.mainWindow }

    public init(manager: PostioUndoManager) {
        self.manager = manager
    }

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

    /// `NSResponder`'s selectors have no AppKit declaration to take `#selector` of, so this type's
    /// own are used; they are the same selectors, so the router hands them on unchanged.
    public static let undoAction = #selector(UndoRouter.undo(_:))
    public static let redoAction = #selector(UndoRouter.redo(_:))
}
