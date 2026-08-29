import AppKit
import PostioFFI
import Testing

@testable import PostioKit

/// A row source that answers from a dictionary, so the table's behaviour can
/// be asserted without a store, a session or the Keychain.
///
/// The *model* — paging, read-ahead, the bounded cache, the generation guard —
/// is tested in Rust where it lives. What is checked here is narrower and
/// entirely Swift's: how many rows the table claims, what it draws for one
/// that has not arrived, and whether it reuses its cells.
final class StubRowSource: MessageRowSource {
    var rowCount: UInt32
    var rows: [UInt32: RowFfi]
    private(set) var asked: [UInt32] = []

    init(rowCount: UInt32, rows: [UInt32: RowFfi] = [:]) {
        self.rowCount = rowCount
        self.rows = rows
    }

    func row(at position: UInt32) -> RowFfi? {
        asked.append(position)
        return rows[position]
    }
}

private func makeRow(
    id: Int64 = 1,
    from: String? = "ada@example.com",
    subject: String? = "A subject",
    preview: String? = "A preview",
    seen: Bool = true,
    flagged: Bool = false,
    threadCount: UInt32 = 1,
    isThread: Bool = false
) -> RowFfi {
    RowFfi(
        id: id,
        thread: nil,
        // The discriminator the verbs need, added on `main` while the macOS
        // branch was out: a thread row's `id` is its newest message, so
        // `thread` being set is not on its own the answer to "is this a
        // conversation row". Defaulted to a message row here because that is
        // what these tests are about.
        isThread: isThread,
        from: from,
        subject: subject,
        preview: preview,
        receivedAt: 0,
        seen: seen,
        flagged: flagged,
        answered: false,
        draft: false,
        hasAttachments: false,
        threadCount: threadCount
    )
}

@MainActor
struct MessageTableTests {
    @Test func theRowCountComesFromTheEngineNotAnArray() {
        // A hundred thousand rows with two of them resident. If the table
        // sized itself from what it holds it would claim two, and the whole
        // windowing arrangement would be pointless.
        let source = StubRowSource(rowCount: 100_000, rows: [0: makeRow(), 1: makeRow()])
        let controller = MessageTableController(source: source)
        let table = NSTableView()

        #expect(controller.numberOfRows(in: table) == 100_000)
    }

    @Test func aRowThatHasNotArrivedDrawsAPlaceholder() {
        // Not a blank. A blank row and a row that is genuinely empty look
        // identical, and only one of them is worth waiting for.
        let source = StubRowSource(rowCount: 10)
        let controller = MessageTableController(source: source)

        let shown = controller.presentation(at: 3)
        #expect(shown.isPlaceholder)
        #expect(shown == .placeholder)
    }

    @Test func aDeliveredRowDrawsItsContents() {
        let source = StubRowSource(
            rowCount: 1,
            rows: [0: makeRow(from: "grace@example.com", subject: "Compiler", seen: false)]
        )
        let controller = MessageTableController(source: source)

        let shown = controller.presentation(at: 0)
        #expect(!shown.isPlaceholder)
        #expect(shown.sender == "grace@example.com")
        #expect(shown.subject == "Compiler")
        #expect(shown.unread)
    }

    @Test func aConversationOfOneShowsNoBadge() {
        // The badge means "there is more here than this" (ADR 0015), so at one
        // it says nothing. A "1" beside every row is noise that reads as data.
        let alone = RowPresentation(row: makeRow(threadCount: 1))
        #expect(alone.threadBadge == nil)

        let several = RowPresentation(row: makeRow(threadCount: 4))
        #expect(several.threadBadge == "4")
    }

    @Test func aMessageWithNoSenderOrSubjectSaysSo() {
        // Both happen. A blank column reads as a rendering failure, which
        // sends the reader looking in the wrong place.
        let bare = RowPresentation(row: makeRow(from: nil, subject: "   ", preview: nil))
        #expect(bare.sender == "(no sender)")
        #expect(bare.subject == "(no subject)")
        #expect(bare.preview.isEmpty)
    }

    @Test func anOfferedCellIsReusedRatherThanRebuilt() throws {
        // Counted, not eyeballed: a table building a fresh view per row
        // scrolls acceptably in a demo and badly in a real mailbox, and
        // nothing about the appearance says which it is doing.
        //
        // This asserts the half that is ours. Whether `NSTableView` offers a
        // view back needs a real row lifecycle and is Apple's to get right;
        // whether we *take* the offer is the part that can be written wrong.
        let controller = MessageTableController(source: StubRowSource(rowCount: 200))

        let made = controller.cell(reusing: nil)
        #expect(controller.cellsCreated == 1)

        let again = controller.cell(reusing: made)
        #expect(again === made, "an offered cell was discarded")
        #expect(
            controller.cellsCreated == 1,
            "a second cell was built for a row that could have reused the first"
        )
    }

    @Test func somethingThatIsNotOneOfOurCellsIsNotReused() throws {
        // AppKit can hand back a view registered under the same identifier by
        // something else. Casting it blindly would crash; ignoring it and
        // building our own is the only safe reading.
        let controller = MessageTableController(source: StubRowSource(rowCount: 1))
        let foreign = NSView()

        let made = controller.cell(reusing: foreign)
        #expect(made is MessageRowCell)
        #expect(controller.cellsCreated == 1)
    }

    @Test func aCellShowsThisRowAndNotTheLastOne() {
        // The classic recycled-cell bug: a cell that only sets the fields it
        // has keeps the previous row's subject under this row's sender. It
        // looks like a data problem and is a drawing one.
        let cell = MessageRowCell()
        cell.show(RowPresentation(row: makeRow(from: "ada@example.com", subject: "First")))
        cell.show(RowPresentation(row: makeRow(from: "grace@example.com", subject: nil)))

        // Nothing of the first row survives into the second.
        let rendered = cell.renderedForTesting
        #expect(rendered.sender == "grace@example.com")
        #expect(rendered.subject == "(no subject)")
        #expect(!rendered.subject.contains("First"))
    }
}
