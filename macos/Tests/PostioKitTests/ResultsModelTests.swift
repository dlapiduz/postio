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
        func focusSearchFile(_ position: UInt64) -> FileCardFfi? {
            fileReads.append(position)
            return position < 3 ? ResultsModelTests.card(position) : nil
        }
        var fileReads: [UInt64] = []
        func focusSearchPerson(_ position: UInt64) -> PersonRowFfi? {
            personReads.append(position)
            return position < 2 ? ResultsModelTests.person(position) : nil
        }
        var personReads: [UInt64] = []
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
                months: [], timelineHint: "Matches by month · drag across months to narrow", timelineStep: nil,
                groups: [
                    group("Top hits", first: 0, rows: 3, topHits: true),
                    group("September 2026", first: 3, rows: 9),
                    group("August 2026", first: 12, rows: 14),
                ],
                rows: 26, cursor: cursor, footerHints: [], footerRight: "26 conversations · local index · 41 ms",
                selected: 0, bulk: [], selectAll: nil, files: nil))
    }

    nonisolated static func card(_ position: UInt64) -> FileCardFfi {
        FileCardFfi(
            attachment: Int64(position) + 70, message: Int64(position) + 1000, kind: "PDF", preview: .page,
            marked: nil, name: [RunFfi(text: "notes.pdf", highlighted: false, style: .plain)], meta: "", line: [],
            subject: "in ‘Notes’", focused: false, accessible: "notes.pdf")
    }

    /// Step 9: the Files tab's frame, three cards.
    static func files(cursor: UInt64? = 0) -> UiEvent {
        .focusResults(
            view: ResultsViewFfi(
                tabs: [TabFfi(tab: .files, label: "Files", count: "3", selected: true, key: "cmd+2")],
                order: .newest, countLine: "26 conversations", subLine: "3 files", months: [], timelineHint: "",
                timelineStep: nil, groups: [], rows: 3, cursor: cursor, footerHints: [], footerRight: "",
                selected: 0, bulk: [], selectAll: nil,
                files: FilesHeaderFfi(title: "Files whose name or contents match", note: "contents are indexed")))
    }

    nonisolated static func person(_ position: UInt64) -> PersonRowFfi {
        PersonRowFfi(
            address: position == 0 ? "ada@example.com" : "tomas@example.com",
            name: position == 0 ? "Ada Moreno" : "Tomás Reyes", initials: position == 0 ? "AM" : "TR",
            messages: "3 messages", last: "26 Sep", focused: position == 0,
            accessible: "Ada Moreno, ada@example.com, 3 messages, last 26 Sep")
    }

    /// Step 10: the People tab's frame, two people.
    static func people(cursor: UInt64? = 0) -> UiEvent {
        .focusResults(
            view: ResultsViewFfi(
                tabs: [
                    TabFfi(tab: .conversations, label: "Conversations", count: "26", selected: false, key: "cmd+1"),
                    TabFfi(tab: .people, label: "People", count: "2", selected: true, key: "cmd+3"),
                ],
                order: .bestMatch, countLine: "26 conversations", subLine: "2 people", months: [],
                timelineHint: "", timelineStep: nil, groups: [], rows: 2, cursor: cursor, footerHints: [],
                footerRight: "", selected: 0, bulk: [], selectAll: nil, files: nil))
    }

    @Test func thePeopleTabIsAListOfPeopleNotTableRows() {
        let engine = Engine()
        let model = ResultsModel(engine: engine)
        model.apply(Self.results())
        #expect(!model.isPeople)
        #expect(model.apply(Self.people()) == .redraw)
        #expect(model.isPeople)
        #expect(!model.isFiles)
        #expect(model.count == 0, "the table draws nothing under the list")
        #expect(model.personCount == 2)
        #expect(model.person(at: 1)?.address == "tomas@example.com")
        #expect(model.person(at: 1)?.address == "tomas@example.com")
        #expect(engine.personReads == [1], "read once, then remembered")
        #expect(model.person(at: 2) == nil)
        model.point(person: 1)
        #expect(engine.pointed == [1])
        model.open(person: 0)
        #expect(engine.pointed == [1, 0])
        #expect(engine.invoked == [ResultsCommand.open], "↩ runs from: them")
        // Back on Conversations, the table draws again.
        model.apply(Self.results())
        #expect(!model.isPeople)
        #expect(model.personCount == 0)
    }

    @Test func theFilesTabIsAGridOfCardsNotTableRows() {
        let engine = Engine()
        let model = ResultsModel(engine: engine)
        model.apply(Self.results())
        #expect(!model.isFiles)
        #expect(model.apply(Self.files()) == .redraw)
        #expect(model.isFiles)
        #expect(model.filesHeader?.title == "Files whose name or contents match")
        #expect(model.count == 0, "the table draws nothing under the grid")
        #expect(model.cardCount == 3)
        #expect(model.card(at: 1)?.attachment == 71)
        #expect(model.card(at: 1)?.attachment == 71)
        #expect(engine.fileReads == [1], "read once, then remembered")
        #expect(model.card(at: 3) == nil)
        // A new frame reads the cards again.
        model.apply(Self.files(cursor: 1))
        _ = model.card(at: 1)
        #expect(engine.fileReads == [1, 1])
        #expect(model.cursor == 1)
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
