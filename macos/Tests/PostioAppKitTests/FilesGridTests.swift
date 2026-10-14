import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// The Files tab's grid (specs/010-focus-search T130; design §3.8, screen
/// 11): four cards across with 12 between them. The ring is the
/// controller's; the arrows are the grid's, as they are any collection
/// view's -- ← and → one card, ↑ and ↓ a row of four -- and say where the
/// ring goes with `focusSearchPoint`, which moves nothing here until the
/// controller says it did.
@MainActor
struct FilesGridTests {
    final class Engine: ResultsEngine {
        var pointed: [UInt64] = []
        var invoked: [String] = []
        var reads: [UInt64] = []

        func focusSearchRow(_ position: UInt64) -> ResultRowFfi? { nil }
        func focusSearchFile(_ position: UInt64) -> FileCardFfi? {
            reads.append(position)
            return FilesGridTests.card(position)
        }
        func focusSearchPerson(_ position: UInt64) -> PersonRowFfi? { nil }
        func focusSearchPoint(_ position: UInt64) { pointed.append(position) }
        func focusSearchTab(_ tab: ResultsTabFfi) {}
        func focusSearchOrder(_ order: ConversationOrderFfi) {}
        func invoke(_ id: String) { invoked.append(id) }
    }

    nonisolated static func card(_ position: UInt64) -> FileCardFfi {
        FileCardFfi(
            attachment: Int64(position) + 70, message: Int64(position) + 1000, kind: "XLSX", preview: .sheet,
            marked: 2, name: [RunFfi(text: "Atlas-Q3-budget.xlsx", highlighted: false, style: .plain)],
            meta: "Ada Moreno · 26 Sep · 48 KB", line: [], subject: "in ‘Re: Atlas Q3 budget’",
            focused: false, accessible: "Atlas-Q3-budget.xlsx")
    }

    static func files(_ count: UInt64, cursor: UInt64?) -> UiEvent {
        .focusResults(
            view: ResultsViewFfi(
                tabs: [], order: .newest, sortable: false, countLine: "12 conversations", subLine: "9 files", months: [],
                timelineHint: "", timelineStep: nil, groups: [], rows: count, cursor: cursor, footerHints: [],
                footerRight: "", selected: 0, bulk: [], selectAll: nil,
                files: FilesHeaderFfi(
                    title: "Files whose name or contents match",
                    note: "contents are indexed on this Mac for PDF, Office documents and text")))
    }

    /// A bare arrow key, as the keyboard sends it.
    static func arrow(_ code: UInt16) -> NSEvent {
        let scalar: Int = switch code {
        case 123: NSLeftArrowFunctionKey
        case 124: NSRightArrowFunctionKey
        case 125: NSDownArrowFunctionKey
        default: NSUpArrowFunctionKey
        }
        let characters = String(Character(UnicodeScalar(scalar)!))
        return NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [.numericPad, .function], timestamp: 0,
            windowNumber: 0, context: nil, characters: characters, charactersIgnoringModifiers: characters,
            isARepeat: false, keyCode: code)!
    }

    static let left: UInt16 = 123
    static let right: UInt16 = 124
    static let down: UInt16 = 125
    static let up: UInt16 = 126

    @Test func theArrowsMoveTheRingByOneAcrossAndByFourDown() {
        let engine = Engine()
        let model = ResultsModel(engine: engine)
        model.apply(Self.files(9, cursor: 1))
        let grid = FilesGrid(model: model)
        #expect(FilesGrid.columns == 4)

        grid.collectionView.keyDown(with: Self.arrow(Self.right))
        grid.collectionView.keyDown(with: Self.arrow(Self.left))
        grid.collectionView.keyDown(with: Self.arrow(Self.down))
        #expect(engine.pointed == [2, 0, 5], "one across, one back, a row of four down")

        // Nothing above the first row: the ring stays, and nothing is said.
        grid.collectionView.keyDown(with: Self.arrow(Self.up))
        #expect(engine.pointed == [2, 0, 5])

        model.apply(.focusResultsCursor(position: 5))
        grid.collectionView.keyDown(with: Self.arrow(Self.up))
        #expect(engine.pointed.last == 1, "a row up")
        model.apply(.focusResultsCursor(position: 6))
        grid.collectionView.keyDown(with: Self.arrow(Self.down))
        #expect(engine.pointed.last == 8, "the last card, in the short row below")
        model.apply(.focusResultsCursor(position: 8))
        let said = engine.pointed.count
        grid.collectionView.keyDown(with: Self.arrow(Self.right))
        grid.collectionView.keyDown(with: Self.arrow(Self.down))
        #expect(engine.pointed.count == said, "nothing past the last card")
    }

    @Test func itLaysFourCardsAcrossWithTwelveBetween() throws {
        let model = ResultsModel(engine: Engine())
        model.apply(Self.files(9, cursor: 0))
        let grid = FilesGrid(model: model)
        grid.scrollView.frame = NSRect(x: 0, y: 0, width: 1440, height: 700)
        grid.collectionView.frame = NSRect(x: 0, y: 0, width: 1440, height: 700)
        grid.collectionView.reloadData()
        let layout = try #require(grid.collectionView.collectionViewLayout as? NSCollectionViewFlowLayout)
        #expect(layout.minimumInteritemSpacing == 12)
        #expect(layout.minimumLineSpacing == 12)
        #expect(grid.collectionView.numberOfItems(inSection: 0) == 9)
        let width = grid.itemSize(forWidth: 1440).width
        // 1440 less 20 each side and three gaps of 12, in four.
        #expect(width == 341, "\(width)")
    }
}
