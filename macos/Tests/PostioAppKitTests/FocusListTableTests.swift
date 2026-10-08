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
        func focusRow(at position: UInt32) -> FocusRowFfi? {
            Int(position) < rows.count ? rows[Int(position)] : nil
        }
    }

    static func row(_ id: Int64, marked: Bool = false) -> FocusRowFfi {
        FocusRowFfi(
            kind: .conversation, id: id, thread: id, sender: "Ada Moreno",
            subject: "Atlas headcount numbers", preview: "Quick one.", time: "12:20",
            dayHeading: "Today · Saturday 26 September", unread: false, countBadge: nil,
            hasAttachments: false, sendState: nil, pills: [],
            marker: marked
                ? MarkerLineFfi(
                    chip: "Question", date: nil, quote: "Do you have the numbers?", status: nil,
                    actions: [FocusRowActionFfi(command: "reply", label: "Reply")])
                : nil)
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
}
