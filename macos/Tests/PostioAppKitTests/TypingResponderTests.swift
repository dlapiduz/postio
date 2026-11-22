import AppKit
import Testing
@testable import PostioAppKit
@testable import PostioKit

@MainActor
@Suite struct TypingResponderTests {
    private func oldIsTyping(_ responder: NSResponder?) -> Bool {
        guard let responder else { return false }
        if responder is NSTextInputClient { return true }
        return (responder as? NSView)?.window?.fieldEditor(false, for: responder) != nil
    }

    private func window(containing view: NSView) -> NSWindow {
        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 200),
            styleMask: [.titled], backing: .buffered, defer: true
        )
        w.contentView?.addSubview(view)
        return w
    }

    @Test func theOldCheckCalledATableTyping_onceAFieldEditorExisted() {
        // `fieldEditor(false, for:)` asks whether the window has a field editor, not whether this
        // responder is a text field; the toolbar search makes one, which once refused every
        // bare-character binding.
        let table = NSTableView()
        let w = window(containing: table)

        #expect(!oldIsTyping(table), "no field editor yet, so nothing is being typed into")

        _ = w.fieldEditor(true, for: NSTextField())

        #expect(
            oldIsTyping(table),
            "the bug did not reproduce; the old check may have been innocent"
        )
        #expect(!TypingResponder.isTyping(table), "a table is not a text field")
    }

    @Test func theFieldEditorIsTyping() {
        let editor = NSTextView()
        editor.isFieldEditor = true
        #expect(TypingResponder.isTyping(editor))
    }

    @Test func aTextFieldIsTyping() {
        #expect(TypingResponder.isTyping(NSTextField()))
    }

    @Test func aTableIsNot() {
        #expect(!TypingResponder.isTyping(NSTableView()))
    }

    @Test func aButtonIsNot() {
        #expect(!TypingResponder.isTyping(NSButton()))
    }

    @Test func nothingFocusedIsNotTyping() {
        #expect(!TypingResponder.isTyping(nil))
    }
    @Test func aSearchFieldIsTyping() {
        // Under-reporting typing makes `a` archive mail while someone types; over-reporting only
        // makes a key dead.
        #expect(TypingResponder.isTyping(NSSearchField()))
    }

    @Test func aFocusedFieldsEditorIsTyping() {
        // A SwiftUI `TextField` in focus is the window's field editor, an `NSTextView`; if that
        // stopped holding, bare letters would fire commands mid-word.
        let field = NSTextField()
        let w = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 200),
            styleMask: [.titled], backing: .buffered, defer: true
        )
        w.contentView?.addSubview(field)
        let editor = w.fieldEditor(true, for: field)
        #expect(editor is NSTextInputClient, "the field editor stopped being an input client")
        #expect(TypingResponder.isTyping(editor))
    }

}
