import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

@MainActor
struct FocusListPointerTests {
    static func table(_ count: Int) -> FocusListTable {
        FocusListTableTests.table((1...count).map { FocusListTableTests.row(Int64($0)) })
    }

    static func ring(_ table: FocusListTable, _ row: Int) -> Bool? {
        table.tableView.layoutSubtreeIfNeeded()
        return (table.tableView.rowView(atRow: row, makeIfNecessary: true) as? FocusRowView)?.isCursor
    }

    @Test func aPlainClickIsReportedAndMovesNothingItself() {
        let table = Self.table(5)
        var pointed: [Int] = []
        table.onPoint = { pointed.append($0) }
        table.clicked(row: 3, modifiers: [])
        #expect(pointed == [3])
        #expect(table.cursor == 0, "the cursor moves when FocusCursor says so")
    }

    @Test func commandClickTogglesAndShiftClickTakesTheRange() {
        let table = Self.table(5)
        var picked: [(Int, Bool)] = []
        var pointed: [Int] = []
        table.onPick = { picked.append(($0, $1)) }
        table.onPoint = { pointed.append($0) }
        table.clicked(row: 2, modifiers: [.command])
        table.clicked(row: 4, modifiers: [.shift])
        #expect(picked.map(\.0) == [2, 4])
        #expect(picked.map(\.1) == [false, true])
        #expect(pointed.isEmpty)
        #expect(table.selected.isEmpty, "the boxes wait for FocusSelection")
    }

    @Test func aDoubleClickOnARowIsReportedAsActivation() {
        let table = Self.table(5)
        var activated: [Int] = []
        table.onActivate = { activated.append($0) }
        #expect(table.tableView.doubleAction != nil, "the table listens for a double-click")
        table.activated(row: 3)
        table.activated(row: -1)
        table.activated(row: 9)
        #expect(activated == [3], "a row, and only a row that is there")
    }

    @Test func aClickOutsideTheRowsIsNotReported() {
        let table = Self.table(2)
        var pointed: [Int] = []
        table.onPoint = { pointed.append($0) }
        table.clicked(row: -1, modifiers: [])
        #expect(pointed.isEmpty)
    }

    @Test func theRingFollowsTheControllersCursor() {
        let table = Self.table(5)
        #expect(Self.ring(table, 0) == true)
        let change = table.model.focus.apply(.focusCursor(position: 2, toTop: false))
        table.apply(change!)
        #expect(Self.ring(table, 0) == false)
        #expect(Self.ring(table, 2) == true)
    }

    @Test func theBoxesFollowTheControllersSelection() {
        let table = Self.table(3)
        let change = table.model.focus.apply(
            .focusSelection(selected: [2], everything: false, summary: "1 selected"))
        table.apply(change!)
        table.tableView.layoutSubtreeIfNeeded()
        let picked = table.tableView.rowView(atRow: 1, makeIfNecessary: true) as? FocusRowView
        let plain = table.tableView.rowView(atRow: 0, makeIfNecessary: true) as? FocusRowView
        #expect(picked?.isPicked == true)
        #expect(plain?.isPicked == false)
    }

    @Test func everythingSelectedBoxesEveryRow() {
        let table = Self.table(3)
        table.apply(table.model.focus.apply(
            .focusSelection(selected: [], everything: true, summary: "All 3 selected"))!)
        table.tableView.layoutSubtreeIfNeeded()
        for row in 0..<3 {
            let view = table.tableView.rowView(atRow: row, makeIfNecessary: true) as? FocusRowView
            #expect(view?.isPicked == true)
        }
    }

    @Test func theSingleHeadingStandsOverTheFirstRowOnly() {
        let table = Self.table(3)
        table.pageArrived(0)
        table.apply(table.model.focus.apply(.focusHeading(text: "Has action \u{b7} 3"))!)
        #expect(table.model.row(at: 0)?.heading == "Has action \u{b7} 3")
        #expect(table.model.row(at: 1)?.heading == nil)
        table.apply(table.model.focus.apply(.focusHeading(text: nil))!)
        #expect(table.model.row(at: 0)?.heading == FocusListTableTests.row(1).dayHeading)
    }

    @Test func standingAtTheTopIsReportedOnlyWhenItChanges() {
        let table = FocusListTableTests.table((1...200).map { FocusListTableTests.row(Int64($0)) })
        var said: [Bool] = []
        table.onAtTop = { said.append($0) }
        table.scrolled()
        table.scrolled()
        table.scrollView.contentView.scroll(to: NSPoint(x: 0, y: 400))
        table.scrolled()
        table.scrolled()
        table.scrollView.contentView.scroll(to: .zero)
        table.scrolled()
        #expect(said == [true, false, true])
    }

    @Test func goingToTheTopScrollsThere() {
        let table = FocusListTableTests.table((1...200).map { FocusListTableTests.row(Int64($0)) })
        table.scrollView.contentView.scroll(to: NSPoint(x: 0, y: 400))
        table.apply(.listToTop)
        #expect(table.scrollView.contentView.bounds.origin.y == 0)
    }
}

@MainActor
struct AttachmentMenuTests {
    static let choices = OpenChoicesFfi(
        links: [OpenLinkFfi(words: "https://example.com", target: "https://example.net/sign-in")],
        parts: [
            FocusAttachmentFfi(id: 41, name: "report.pdf", size: "48 KB", saveName: "report.pdf"),
            FocusAttachmentFfi(id: 42, name: "photo.png", size: "2 MB", saveName: "photo.png"),
        ],
        nothing: nil, save: "Save", saveAll: "Save all")

    private func choose(_ title: String, in menu: NSMenu) {
        guard let item = menu.items.first(where: { $0.title == title }) else {
            Issue.record("no item \(title)")
            return
        }
        _ = item.target?.perform(item.action, with: item)
    }

    @Test func theLinksComeFirstThenThePartsThenEachPartAgainUnderSave() {
        let built = AttachmentMenu { _ in }.menu(for: Self.choices)
        #expect(
            built.items.map(\.title) == [
                "https://example.com", "", "report.pdf  48 KB", "photo.png  2 MB", "", "Save",
                "report.pdf\u{2026}", "photo.png\u{2026}", "Save all\u{2026}",
            ])
        #expect(built.items[0].toolTip == "https://example.net/sign-in", "where the link goes is seen before it opens")
        if #available(macOS 14.4, *) {
            #expect(built.items[0].subtitle == "https://example.net/sign-in")
        }
    }

    @Test func eachItemSaysWhatWasChosen() {
        var chosen: [AttachmentMenu.Choice] = []
        let menu = AttachmentMenu { chosen.append($0) }
        let built = menu.menu(for: Self.choices)
        choose("https://example.com", in: built)
        choose("photo.png  2 MB", in: built)
        choose("report.pdf\u{2026}", in: built)
        choose("Save all\u{2026}", in: built)
        #expect(
            chosen == [
                .link("https://example.net/sign-in"), .open(42), .save(id: 41, name: "report.pdf"), .saveAll,
            ])
    }

    @Test func aMessageWithOnlyLinksOffersNothingToSave() {
        let built = AttachmentMenu { _ in }.menu(
            for: OpenChoicesFfi(
                links: Self.choices.links, parts: [], nothing: nil, save: "Save", saveAll: "Save all"))
        #expect(built.items.map(\.title) == ["https://example.com"])
    }
}
