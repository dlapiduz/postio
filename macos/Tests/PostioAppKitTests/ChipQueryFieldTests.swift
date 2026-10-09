import AppKit
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// The results' query field (specs/010-focus-search T069; design §1 and
/// §3.1; research R11): an `NSTextView` whose chips are text attachments,
/// so free words can sit after them and an excluded chip can be struck
/// through, which `NSTokenField` can do neither of.
///
/// It draws what `FocusQuery` said and reports what the keyboard did: a
/// chip removed by its token, a wish to edit the query. It never edits the
/// query itself -- the next `FocusQuery` redraws it.
@MainActor
@Suite struct ChipQueryFieldTests {
    static func chip(_ token: UInt32, _ op: String, _ value: String, excluded: Bool = false)
        -> SearchQueryModel.Chip
    {
        SearchQueryModel.Chip(id: token, op: op, value: value, excluded: excluded, focused: false)
    }

    static func field() -> ChipQueryField {
        let field = ChipQueryField(frame: NSRect(x: 0, y: 0, width: 600, height: 34))
        field.show(
            chips: [chip(5, "from:", "ada"), chip(2, "has:", "attachment"), chip(9, "in:", "Archive", excluded: true)],
            words: "atlas budget")
        return field
    }

    static func window(containing view: NSView) -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 80),
            styleMask: [.titled], backing: .buffered, defer: true)
        window.contentView?.addSubview(view)
        return window
    }

    @Test func chipsAreDrawnBeforeTheWordsInTokenOrder() {
        let field = Self.field()

        #expect(field.chipAttachments.map(\.chip.id) == [5, 2, 9])
        #expect(field.words == "atlas budget")
        // The attachments come first in the text, one character each.
        let text = field.textStorage?.string ?? ""
        #expect(text.utf16.prefix(3).allSatisfy { $0 == 0xFFFC })
        #expect(text.hasSuffix("atlas budget"))
    }

    @Test func anExcludedChipIsStruckThrough() {
        let field = Self.field()
        let struck = field.chipAttachments.map { attachment -> Bool in
            let label = attachment.label
            var any = false
            label.enumerateAttribute(.strikethroughStyle, in: NSRange(location: 0, length: label.length)) { value, _, _ in
                if let style = value as? Int, style != 0 { any = true }
            }
            return any
        }
        #expect(struck == [false, false, true])
    }

    @Test func backspaceAtTheStartOfTheWordsSelectsTheLastChipThenRemovesIt() {
        let field = Self.field()
        var removed: [UInt32] = []
        field.onRemove = { removed.append($0) }
        field.setSelectedRange(NSRange(location: 3, length: 0))

        field.deleteBackward(nil)
        #expect(field.selectedRange() == NSRange(location: 2, length: 1), "the last chip is selected")
        #expect(removed.isEmpty)

        field.deleteBackward(nil)
        #expect(removed == [9], "one Remove, by its token")
        // Nothing edited here: the controller's next `FocusQuery` redraws.
        #expect(field.chipAttachments.count == 3)
    }

    @Test func slashFocusesTheFieldAndAsksForTheDropdown() throws {
        let field = Self.field()
        let window = Self.window(containing: field)
        var asked = 0
        field.onFocus = { asked += 1 }

        let slash = try #require(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: window.windowNumber, context: nil, characters: "/",
            charactersIgnoringModifiers: "/", isARepeat: false, keyCode: 44))
        field.focusAndAsk()
        #expect(window.firstResponder === field)
        #expect(asked == 1)

        field.keyDown(with: slash)
        #expect(asked == 2, "typed into the field, / asks for the dropdown too")
        #expect(field.words == "atlas budget", "and is not typed into the words")
    }

    @Test func typingIntoTheFieldIsHandedOnRatherThanEditedHere() {
        let field = Self.field()
        var typed: [String] = []
        field.onType = { typed.append($0) }
        field.setSelectedRange(NSRange(location: field.textStorage?.length ?? 0, length: 0))

        field.insertText(" q3", replacementRange: field.selectedRange())

        #expect(typed == [" q3"])
        #expect(field.words == "atlas budget")
    }
}
