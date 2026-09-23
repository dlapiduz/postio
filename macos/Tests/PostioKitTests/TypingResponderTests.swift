import AppKit
import Testing
@testable import PostioKit

@MainActor
@Suite struct TypingResponderTests {
    /// The old predicate, kept here so the bug it had can be demonstrated
    /// rather than described.
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
        // The reported bug: `/` did nothing. `fieldEditor(false, for:)` asks
        // whether the *window* has a field editor, not whether this responder
        // is a text field — and the search box in the toolbar makes one. From
        // then on every bare-character binding was refused, because the
        // monitor believed the message list was being typed into.
        let table = NSTableView()
        let w = window(containing: table)

        #expect(!oldIsTyping(table), "no field editor yet, so nothing is being typed into")

        // What focusing the search box does.
        _ = w.fieldEditor(true, for: NSTextField())

        #expect(
            oldIsTyping(table),
            "the bug did not reproduce; the old check may have been innocent"
        )
        #expect(!TypingResponder.isTyping(table), "a table is not a text field")
    }

    @Test func theFieldEditorIsTyping() {
        // What a focused text field actually makes first responder.
        let editor = NSTextView()
        editor.isFieldEditor = true
        #expect(TypingResponder.isTyping(editor))
    }

    @Test func aTextFieldIsTyping() {
        // The case the old third line said it was for: the field itself,
        // before its editor has been installed.
        #expect(TypingResponder.isTyping(NSTextField()))
    }

    @Test func aTableIsNot() {
        // `j`, `k`, `a` and `/` all have to keep working over the message
        // list, whatever else the window has been used for.
        #expect(!TypingResponder.isTyping(NSTableView()))
    }

    @Test func aButtonIsNot() {
        #expect(!TypingResponder.isTyping(NSButton()))
    }

    @Test func nothingFocusedIsNotTyping() {
        #expect(!TypingResponder.isTyping(nil))
    }
    @Test func aSearchFieldIsTyping() {
        // The dangerous direction. Over-reporting typing makes a key dead;
        // *under*-reporting it makes `a` archive mail while somebody types
        // "already replied", which is the failure the boundary's own doc
        // calls the most visible one this seam can have. `NSSearchField` is
        // what a toolbar search is, and it is an `NSTextField`.
        #expect(TypingResponder.isTyping(NSSearchField()))
    }

    @Test func aFocusedFieldsEditorIsTyping() {
        // What actually holds focus while somebody types into a SwiftUI
        // `TextField`: not the field, but the window's field editor — an
        // `NSTextView`, and so an `NSTextInputClient`. If this ever stopped
        // being true, bare letters would start firing commands mid-word.
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
