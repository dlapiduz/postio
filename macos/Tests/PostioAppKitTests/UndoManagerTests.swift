import AppKit
import Testing

@testable import PostioAppKit

/// Edit › Undo over the engine's stack (specs/009-focus-macos T048, T051,
/// R7).
///
/// The engine's `UndoStack` is the only stack (FR-041). This manager holds
/// nothing but the words for its top, read from the engine, and sends
/// `undo` when chosen; registering a mirror of each action would drift the
/// moment the pill's Undo, an expiry or another window changed the stack.
@MainActor
struct UndoManagerTests {
    final class Sent {
        var count = 0
    }

    static func manager(_ sent: Sent = Sent()) -> PostioUndoManager {
        PostioUndoManager { sent.count += 1 }
    }

    @Test func withNothingToUndoTheItemIsOff() {
        let undo = Self.manager()
        #expect(undo.canUndo == false)
        #expect(undo.undoMenuItemTitle == "Undo")
    }

    @Test func theItemNamesWhatItWouldTakeBack() {
        let undo = Self.manager()
        undo.remember("Archived 3 messages")
        #expect(undo.canUndo)
        #expect(undo.undoActionName == "Archived 3 messages")
        #expect(undo.undoMenuItemTitle == "Undo Archived 3 messages")
    }

    @Test func undoSendsTheEnginesUndoAndNothingElse() {
        let sent = Sent()
        let undo = Self.manager(sent)
        undo.remember("Archived 3 messages")
        undo.undo()
        #expect(sent.count == 1)
    }

    @Test func thereIsNoRedo() {
        // The engine has none, so the item is never offered.
        let undo = Self.manager()
        undo.remember("Archived 3 messages")
        #expect(undo.canRedo == false)
        let sent = Sent()
        let other = Self.manager(sent)
        other.redo()
        #expect(sent.count == 0)
    }

    @Test func refreshingReadsTheEnginesWordsOffTheMainActor() async {
        let undo = Self.manager()
        await undo.refresh { "Snoozed 2 messages" }
        #expect(undo.undoMenuItemTitle == "Undo Snoozed 2 messages")
        await undo.refresh { nil }
        #expect(undo.canUndo == false)
    }

    @Test func anOlderReadNeverOverwritesANewerOne() async {
        // Two toasts in a burst: the read that started second is the truer
        // one, whichever lands last.
        let undo = Self.manager()
        let slow = Task { await undo.refresh { Thread.sleep(forTimeInterval: 0.2); return "Archived" } }
        try? await Task.sleep(nanoseconds: 20_000_000)
        await undo.refresh { "Snoozed" }
        await slow.value
        #expect(undo.undoActionName == "Snoozed")
    }

    @Test func textFieldsKeepTheirOwnUndo() {
        // Typing in a field undoes typing: the engine's manager is only the
        // main window's while nothing there takes text.
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled],
            backing: .buffered, defer: true)
        KeyWindowTracker.tag(window, as: .main)
        let undo = Self.manager()
        let router = UndoRouter(manager: undo)
        #expect(router.engineUndo(in: window) === undo)
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 100, height: 20))
        window.contentView?.addSubview(field)
        let editor = NSTextView()
        #expect(router.engineUndo(in: window, firstResponder: editor) == nil)
        #expect(router.engineUndo(in: window, firstResponder: field) == nil)
    }

    @Test func anotherWindowKeepsItsOwnUndo() {
        let compose = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled],
            backing: .buffered, defer: true)
        KeyWindowTracker.tag(compose, as: .compose, draft: 7)
        let router = UndoRouter(manager: Self.manager())
        #expect(router.engineUndo(in: compose) == nil)
        #expect(router.engineUndo(in: nil) == nil)
    }

    @Test func theMenusUndoItemNamesTheEnginesTop() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 100), styleMask: [.titled],
            backing: .buffered, defer: true)
        KeyWindowTracker.tag(window, as: .main)
        let undo = Self.manager()
        undo.remember("Archived 3 messages")
        let router = UndoRouter(manager: undo)
        router.window = { window }
        let item = NSMenuItem(title: "Undo", action: UndoRouter.undoAction, keyEquivalent: "z")
        #expect(NSStringFromSelector(UndoRouter.undoAction) == "undo:")
        #expect(router.validateMenuItem(item))
        #expect(item.title == "Undo Archived 3 messages")
        undo.remember(nil)
        #expect(router.validateMenuItem(item) == false)
        #expect(item.title == "Undo")
    }

    @Test func editsUndoAndRedoAreAimedAtTheRouterWithTheirChords() {
        // ⌘Z in a field reaches the field through this item, and in the
        // list the engine's stack; ⇧⌘Z likewise.
        let router = UndoRouter(manager: Self.manager())
        let menu = NSMenu(title: "Edit")
        MenuBar.appendStandardEditing(to: menu, undo: router)
        let undo = menu.items.first { $0.action == UndoRouter.undoAction }
        let redo = menu.items.first { $0.action == UndoRouter.redoAction }
        #expect(undo?.target === router)
        #expect(undo?.keyEquivalent == "z")
        #expect(redo?.target === router)
        #expect(redo?.keyEquivalentModifierMask == [.command, .shift])
    }
}
