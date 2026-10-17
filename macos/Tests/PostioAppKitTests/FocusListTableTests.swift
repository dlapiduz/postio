import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// What the table decides on its own, without a store: that it never
/// selects, that its heights are the model's, that it reuses its cells,
/// and that the cursor's ring is on the first row when a list opens.
@MainActor
struct FocusListTableTests {
    final class Rows: FocusRowSource {
        var rows: [FocusRowFfi?]
        init(_ rows: [FocusRowFfi?]) { self.rows = rows }
        var focusRowCount: UInt32 { UInt32(rows.count) }
        func residentFocusRow(at position: UInt32) -> FocusRowFfi? {
            Int(position) < rows.count ? rows[Int(position)] : nil
        }
        func focusRow(at position: UInt32) -> FocusRowFfi? {
            Int(position) < rows.count ? rows[Int(position)] : nil
        }
    }

    static func row(_ id: Int64, marked: Bool = false) -> FocusRowFfi {
        FocusRowFfi(
            kind: .conversation, id: id, thread: id, threads: [id], sender: "Ada Moreno",
            subject: "Atlas headcount numbers", preview: "Quick one.", time: "12:20",
            dayHeading: "Today · Saturday 26 September", unread: false, countBadge: nil,
            hasAttachments: false, sendState: nil, pills: [],
            marker: marked
                ? MarkerLineFfi(
                    chip: "Question", date: nil, quote: "Do you have the numbers?", status: nil,
                    actions: [FocusRowActionFfi(command: "reply", label: "Reply")])
                : nil,
            writes: false)
    }

    static func table(_ rows: [FocusRowFfi?]) -> FocusListTable {
        let model = FocusListModel(source: Rows(rows)) { $0 == "reply" ? "e" : nil }
        let table = FocusListTable(model: model)
        table.scrollView.frame = NSRect(x: 0, y: 0, width: 1440, height: 800)
        table.listChanged(total: UInt32(rows.count))
        return table
    }

    @Test func theTableNeverSelectsARow() {
        let table = Self.table([Self.row(1), Self.row(2)])
        #expect(table.tableView(table.tableView, shouldSelectRow: 0) == false)
        #expect(table.tableView.selectionHighlightStyle == .none)
        #expect(table.tableView.usesAutomaticRowHeights == false)
    }

    @Test func aRowsHeightIsItsShapesOnceItsPageHasArrived() {
        let table = Self.table([Self.row(1), Self.row(2, marked: true)])
        table.pageArrived(0)
        #expect(table.tableView(table.tableView, heightOfRow: 0) == 72)  // headed one-liner
        #expect(table.tableView(table.tableView, heightOfRow: 1) == 72)  // two lines
    }

    @Test func cellsAreReused() {
        let table = Self.table([Self.row(1)])
        let first = table.cell(reusing: nil)
        let again = table.cell(reusing: first)
        #expect(first === again)
        #expect(table.cellsCreated == 1)
    }

    @Test func theCursorOpensOnTheFirstRow() {
        // C30: every list opens with the cursor on its first row.
        let table = Self.table([Self.row(1), Self.row(2)])
        #expect(table.cursor == 0)
        table.tableView.layoutSubtreeIfNeeded()
        let ring = table.tableView.rowView(atRow: 0, makeIfNecessary: true) as? FocusRowView
        let plain = table.tableView.rowView(atRow: 1, makeIfNecessary: true) as? FocusRowView
        #expect(ring?.isCursor == true)
        #expect(plain?.isCursor == false)
    }

    @Test func anActionsKeycapIsTheBindings() {
        let table = Self.table([Self.row(1, marked: true)])
        let cell = table.tableView(table.tableView, viewFor: nil, row: 0) as? FocusRowCell
        #expect(cell?.shown?.marker?.actions.first?.cap == "e")
    }

    @Test func aPickerHangsUnderItsRowAtTheSubjectColumn() {
        // Screen 11: the picker's left edge on the subject column, its top
        // under the row's own lines -- not under its day heading's band.
        let table = Self.table([Self.row(1), Self.row(2)])
        table.pageArrived(0)
        table.tableView.layoutSubtreeIfNeeded()
        let width = table.tableView.bounds.width
        let subject = CGFloat(FocusRowMetrics.columns(width: Double(width)).subjectX)
        let first = table.pickerAnchor(row: 0, width: 380)
        #expect(first == NSRect(x: subject, y: 32, width: 380, height: 40))
        let second = table.pickerAnchor(row: 1, width: 380)
        #expect(second == NSRect(x: subject, y: 72, width: 380, height: 40))
        #expect(table.pickerAnchor(row: 5, width: 380) == nil)
    }

    /// A long list of mixed rows: every third is a two-liner, and the day
    /// changes every four rows.
    static func mixed(_ ids: [Int64]) -> [FocusRowFfi?] {
        ids.map { id in
            var made = row(id, marked: id % 3 == 0)
            made.dayHeading = "Day \(id / 4)"
            return made
        }
    }

    @Test func archivingTheCursorsRowNeverDrawsAFrameOfUnmeasuredRows() {
        // The recorded glitch: `FocusListChanged` arrives with the page
        // that shrank the list already resident, and the table laid out
        // once at one-line heights -- the scroll jumped, the second lines
        // vanished and day headings were bands over bare rows -- before
        // `FocusPageReady` set it right. Nothing between the two events may
        // be drawn.
        let before = Array(0..<60 as Range<Int64>)
        let source = Rows(Self.mixed(before))
        let model = FocusListModel(source: source) { _ in nil }
        let table = FocusListTable(model: model)
        table.scrollView.frame = NSRect(x: 0, y: 0, width: 1440, height: 800)
        table.listChanged(total: 60)
        table.pageArrived(0)
        table.pageArrived(1)
        table.tableView.layoutSubtreeIfNeeded()
        model.focus.apply(.focusCursor(position: 30, toTop: false))
        table.apply(.cursor(previous: 0, toTop: false))
        table.tableView.layoutSubtreeIfNeeded()
        let ringBefore = table.tableView.rect(ofRow: 30).minY - table.scrollView.contentView.bounds.minY

        // Archive row 30: the page is re-read, shorter, and the count event
        // lands first.
        source.rows = Self.mixed(before.filter { $0 != 30 })
        table.listChanged(total: 59)
        table.tableView.layoutSubtreeIfNeeded()

        // The truth, as a model that has seen every row would say it.
        let truth = FocusListModel(source: source) { _ in nil }
        truth.reset(total: 59)
        _ = truth.pageArrived(0)
        _ = truth.pageArrived(1)
        let visible = table.tableView.rows(in: table.tableView.visibleRect)
        #expect(visible.length > 0)
        for row in visible.location..<(visible.location + visible.length) {
            #expect(table.tableView.rect(ofRow: row).height == CGFloat(truth.height(at: row)), "row \(row)")
        }
        // The ring stays where the eye had it: the next row slid up under it.
        let ringAfter = table.tableView.rect(ofRow: 30).minY - table.scrollView.contentView.bounds.minY
        #expect(abs(ringAfter - ringBefore) < 1)
    }
}
