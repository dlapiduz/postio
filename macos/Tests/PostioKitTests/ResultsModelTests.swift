import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The results table's model (specs/010-focus-search T071; design §3.4,
/// screens 06 and 07).
///
/// The controller decides what the results are, in which groups, and
/// where the focus ring is (`crates/postio-focus/src/results.rs`); the
/// frame arrives as `FocusResults`, the rows are read by position, and a
/// page that landed is `FocusResultsPage`. What the Mac works out is only
/// the table's own shape: a header row before each group's rows, each
/// row's height from its group, and which rows to read again.
@MainActor
struct ResultsModelTests {
    final class Engine: ResultsEngine {
        var reads: [UInt64] = []
        var pointed: [UInt64] = []
        var invoked: [String] = []
        var pending: Set<UInt64> = []

        func focusSearchRow(_ position: UInt64) -> ResultRowFfi? {
            reads.append(position)
            return pending.contains(position) ? nil : ResultsModelTests.row(position)
        }
        func focusSearchPoint(_ position: UInt64) { pointed.append(position) }
        func focusSearchTab(_ tab: ResultsTabFfi) {}
        func focusSearchOrder(_ order: ConversationOrderFfi) {}
        func invoke(_ id: String) { invoked.append(id) }
    }

    nonisolated static func row(_ position: UInt64) -> ResultRowFfi {
        ResultRowFfi(
            id: Int64(position) + 100, thread: nil, group: 0, topHit: position < 3, sender: "Ada Moreno",
            unread: false, reason: nil, subject: [RunFfi(text: "Atlas", highlighted: true, style: .plain)],
            pills: [], attachments: false, countBadge: nil, sourceTag: "body", sourceIsFile: false,
            passage: [], folder: "in:Inbox", date: "26 Sep", checked: false, accessible: "Ada Moreno")
    }

    static func group(_ title: String, first: UInt64, rows: UInt64, topHits: Bool = false) -> ResultGroupFfi {
        ResultGroupFfi(
            title: title, count: String(rows), note: nil, first: first, rows: rows, topHits: topHits,
            accessible: "\(title) · \(rows)")
    }

    /// Screen 06's shape: three Top hits, then September's nine and
    /// August's fourteen.
    static func results(cursor: UInt64? = 0) -> UiEvent {
        .focusResults(
            view: ResultsViewFfi(
                tabs: [TabFfi(tab: .conversations, label: "Conversations", count: "26", selected: true, key: "cmd+1")],
                order: .bestMatch, countLine: "26 conversations", subLine: "12 files · 6 people · last 12 months",
                months: [], groups: [
                    group("Top hits", first: 0, rows: 3, topHits: true),
                    group("September 2026", first: 3, rows: 9),
                    group("August 2026", first: 12, rows: 14),
                ],
                rows: 26, cursor: cursor, footerHints: [], footerRight: "26 conversations · local index · 41 ms",
                selected: 0, bulk: []))
    }

    @Test func groupHeadersPrecedeTheirRows() {
        let model = ResultsModel(engine: Engine())
        #expect(model.apply(Self.results()) == .open)

        #expect(model.count == 3 + 9 + 14 + 3)
        #expect(model.item(at: 0) == .header(0))
        #expect(model.item(at: 1) == .row(0))
        #expect(model.item(at: 3) == .row(2))
        #expect(model.item(at: 4) == .header(1))
        #expect(model.item(at: 5) == .row(3))
        #expect(model.item(at: 14) == .header(2))
        #expect(model.item(at: 15) == .row(12))
        #expect(model.item(at: 28) == .row(25))
        #expect(model.item(at: 29) == nil)
        #expect(model.tableRow(of: 12) == 15)
    }

    @Test func topHitsRowsAre66TallOthers58AndHeaders32() {
        let model = ResultsModel(engine: Engine())
        model.apply(Self.results())

        #expect(model.height(at: 0) == 32)
        #expect(model.height(at: 1) == 66)
        #expect(model.height(at: 3) == 66)
        #expect(model.height(at: 4) == 32)
        #expect(model.height(at: 5) == 58)
        #expect(model.height(at: 28) == 58)
    }

    @Test func aResultsPageRereadsOnlyItsRange() {
        let engine = Engine()
        let model = ResultsModel(engine: engine)
        model.apply(Self.results())
        for position in 0..<10 { _ = model.row(at: UInt64(position)) }
        engine.reads = []

        // Positions 2, 3 and 4 sit at table rows 3, 5 and 6 (September's
        // header is row 4).
        #expect(model.apply(.focusResultsPage(first: 2, count: 3)) == .rows(IndexSet([3, 5, 6])))
        for position in 0..<10 { _ = model.row(at: UInt64(position)) }

        #expect(engine.reads == [2, 3, 4])
    }

    @Test func aPendingRowIsAskedForAgainNotRemembered() {
        let engine = Engine()
        engine.pending = [5]
        let model = ResultsModel(engine: engine)
        model.apply(Self.results())

        #expect(model.row(at: 5) == nil)
        engine.pending = []
        #expect(model.row(at: 5)?.id == 105)
        #expect(engine.reads == [5, 5])
    }

    @Test func theCursorMovesWhereTheControllerSays() {
        let model = ResultsModel(engine: Engine())
        model.apply(Self.results(cursor: 0))

        #expect(model.apply(.focusResultsCursor(position: 4)) == .cursor(previous: 1))
        #expect(model.cursor == 4)
        #expect(model.cursorRow == 6)
    }

    @Test func aClickIsToldAndAHeaderIsNot() {
        let engine = Engine()
        let model = ResultsModel(engine: engine)
        model.apply(Self.results())

        model.point(tableRow: 4)
        model.point(tableRow: 5)
        model.open(tableRow: 6)

        #expect(engine.pointed == [3, 4])
        #expect(engine.invoked == ["open_message"])
        #expect(model.cursor == 0, "the ring moves when the controller says, not here")
    }

    @Test func aNewFrameForgetsTheRowsReadAndLeavingCloses() {
        let engine = Engine()
        let model = ResultsModel(engine: engine)
        model.apply(Self.results())
        _ = model.row(at: 1)

        #expect(model.apply(Self.results()) == .redraw)
        _ = model.row(at: 1)
        #expect(engine.reads == [1, 1])

        #expect(model.apply(.focusLeaveResults) == .close)
        #expect(!model.isOpen)
        #expect(model.count == 0)
    }
}
