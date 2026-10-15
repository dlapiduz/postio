import PostioFFI
import Testing

@testable import PostioKit

/// The message window's column follows the window the person resized
/// (spec 009 M1; the pack's message-window SPEC §1): the document's
/// `columnWidth` is only the width it opened at.
struct MessageColumnTests {
    static let opened = MessageWindowModelTests.document(shown: .appColours)

    @Test func aWidenedWindowReadsWider() {
        let opened = Self.opened
        let wider = MessageColumn.at(window: 1080, document: opened)
        #expect(wider.columnWidth == 960)
        #expect(wider.columnWidth > opened.columnWidth)
        #expect(!wider.foldsIntoMore)
    }

    @Test func aNarrowedWindowReadsNarrowerAndFolds() {
        let narrower = MessageColumn.at(window: 680, document: Self.opened)
        #expect(narrower.columnWidth == 600)
        #expect(narrower.foldsIntoMore)
    }

    @Test func paperFollowsTheWindowToo() {
        let paper = MessageWindowModelTests.document(shown: .paper)
        #expect(MessageColumn.at(window: 1080, document: paper).columnWidth == 1000)
    }

    @Test func beforeTheWindowHasAWidthTheDocumentsColumnStands() {
        let column = MessageColumn.at(window: 0, document: Self.opened)
        #expect(column.columnWidth == Self.opened.columnWidth)
        #expect(column.foldsIntoMore == Self.opened.foldsIntoMore)
    }
}
