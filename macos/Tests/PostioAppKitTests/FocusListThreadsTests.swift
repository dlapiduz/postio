import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// A thread opened in place, as the table draws it (spec 013 US1, T033).
///
/// What opens it, where the cursor goes and what the children say are the
/// controller's (`postio-focus`, and `focus_threads` in the FFI suite); this
/// pins what the Mac does with the answer: the rows are inserted under the
/// thread's row and removed again, the keyboard's ring never leaves the row
/// it was on, and the children are drawn as children.
@MainActor
struct FocusListThreadsTests {
    /// A list the way the FFI hands it over: the rows, and the open
    /// thread's children among them (display positions).
    final class Rows: FocusRowSource {
        var rows: [FocusRowFfi]
        init(_ rows: [FocusRowFfi]) { self.rows = rows }
        var focusRowCount: UInt32 { UInt32(rows.count) }
        func residentFocusRow(at position: UInt32) -> FocusRowFfi? {
            Int(position) < rows.count ? rows[Int(position)] : nil
        }
        func focusRow(at position: UInt32) -> FocusRowFfi? { residentFocusRow(at: position) }

        func open(under anchor: Int, children: [FocusRowFfi]) {
            rows.insert(contentsOf: children, at: anchor + 1)
        }
        func fold(under anchor: Int, children: Int) {
            rows.removeSubrange((anchor + 1)..<(anchor + 1 + children))
        }
    }

    static func row(_ id: Int64, thread: Bool = false) -> FocusRowFfi {
        FocusRowFfi(
            kind: .conversation, id: id, thread: id, threads: [id], sender: "Ben Adeyemi",
            fromAddress: "ben@example.org", subject: "Harbor API draft v3",
            preview: "Looks good.", time: "16:40", dayHeading: "Today", unread: thread,
            countBadge: thread ? "6" : nil, hasAttachments: false, sendState: nil, pills: [],
            marker: nil, writes: false,
            threadRow: thread
                ? ThreadRowFfi(
                    participants: "Lena, Ben, Ana, you", count: "6",
                    latestLine: "Ben: Looks good\u{2026}", new: "2 new", chevron: .collapsed)
                : nil,
            child: nil)
    }

    static func child(_ line: String, unread: Bool = false, draft: Bool = false) -> FocusRowFfi {
        FocusRowFfi(
            kind: .child, id: 2, thread: 2, threads: [], sender: draft ? "Draft \u{b7} you" : "Ana Ruiz",
            fromAddress: nil, subject: "", preview: line, time: "Mon", dayHeading: "Today",
            unread: unread, countBadge: nil, hasAttachments: false, sendState: nil, pills: [],
            marker: nil, writes: false, threadRow: nil,
            child: ChildRowFfi(
                initials: "AR", sender: draft ? "Draft \u{b7} you" : "Ana Ruiz", bold: unread,
                tag: draft ? "not sent" : nil, line: line, placeholder: false, attachment: false,
                time: "Mon", draft: draft,
                hint: draft ? nil : ChildHintFfi(keys: ["r", "f"], label: "this one")))
    }

    /// A table over `rows`, with the first page landed.
    static func table(_ rows: Rows) -> FocusListTable {
        let model = FocusListModel(source: rows) { _ in nil }
        let table = FocusListTable(model: model)
        table.scrollView.frame = NSRect(x: 0, y: 0, width: 1440, height: 800)
        table.listChanged(total: UInt32(rows.rows.count))
        table.pageArrived(0)
        table.tableView.layoutSubtreeIfNeeded()
        return table
    }

    /// What the controller says after `Right` on the thread row: the
    /// children are spliced in under it, and the cursor stays on it.
    static func right(_ table: FocusListTable, _ rows: Rows, anchor: Int, children: [FocusRowFfi]) {
        rows.open(under: anchor, children: children)
        table.splice(anchor: anchor, children: children.count)
        table.model.focus.apply(.focusCursor(position: UInt32(anchor), toTop: false))
        table.apply(.cursor(previous: anchor, toTop: false))
        table.tableView.layoutSubtreeIfNeeded()
    }

    /// And after `Left`.
    static func left(_ table: FocusListTable, _ rows: Rows, anchor: Int, children: Int) {
        rows.fold(under: anchor, children: children)
        table.unsplice()
        table.model.focus.apply(.focusCursor(position: UInt32(anchor), toTop: false))
        table.apply(.cursor(previous: anchor, toTop: false))
        table.tableView.layoutSubtreeIfNeeded()
    }

    static let kids = [Self.child("Kicking off v3."), Self.child("Cursor it is.", unread: true),
                       Self.child("Thanks both.", draft: true)]

    private func isCursor(_ table: FocusListTable, _ row: Int) -> Bool? {
        (table.tableView.rowView(atRow: row, makeIfNecessary: true) as? FocusRowView)?.isCursor
    }

    @Test func focusStaysOnTheRowAfterRightAndReturnsToItAfterLeft() {
        let rows = Rows([Self.row(1), Self.row(2, thread: true), Self.row(3)])
        let table = Self.table(rows)
        table.model.focus.apply(.focusCursor(position: 1, toTop: false))
        table.apply(.cursor(previous: 0, toTop: false))
        #expect(isCursor(table, 1) == true)

        Self.right(table, rows, anchor: 1, children: Self.kids)
        #expect(table.tableView.numberOfRows == 6, "three rows and the thread's three children")
        #expect(table.cursor == 1, "the keyboard did not move")
        #expect(isCursor(table, 1) == true, "the ring is on the thread's row")
        #expect(isCursor(table, 2) == false, "and not on the first child")
        #expect(isCursor(table, 5) == false, "nor on the row that slid down")

        Self.left(table, rows, anchor: 1, children: 3)
        #expect(table.tableView.numberOfRows == 3)
        #expect(table.cursor == 1)
        #expect(isCursor(table, 1) == true, "the ring is back on the row it left")
    }

    @Test func theChildrenAreDrawnAsChildrenUnderTheirRow() {
        let rows = Rows([Self.row(1), Self.row(2, thread: true), Self.row(3)])
        let table = Self.table(rows)
        Self.right(table, rows, anchor: 1, children: Self.kids)

        let heights = (0..<6).map { table.tableView(table.tableView, heightOfRow: $0) }
        // The first row starts the day (32 + 40), the thread is 46, each
        // child 38, and the row after them is a plain one-liner.
        #expect(heights == [72, 46, 38, 38, 38, 40])
        let model = table.model.row(at: 4)
        #expect(model?.child?.draft == true, "the unsent reply is last")
        #expect(model?.heading == nil, "no day heading between a thread and its children")
        let marks = (2...4).map {
            (table.tableView.rowView(atRow: $0, makeIfNecessary: true) as? FocusRowView)?.child
        }
        #expect(marks.map { $0?.unread } == [false, true, false], "the dot is accent when unread")
        #expect(marks.map { $0?.last } == [false, false, true], "the line stops at the last dot")
        let thread = table.model.row(at: 1)
        #expect(thread?.chevron == .collapsed, "the gutter's chevron comes from the FFI")
        #expect(thread?.newBadge == "2 new")
        #expect(thread?.sender == "Lena, Ben, Ana, you")
    }

    @Test func aChildAfterTheFocusedOneShowsItsKeys() {
        let rows = Rows([Self.row(2, thread: true)])
        let table = Self.table(rows)
        Self.right(table, rows, anchor: 0, children: Self.kids)
        table.model.focus.apply(.focusCursor(position: 2, toTop: false))
        let shown = table.model.row(at: 2)
        #expect(shown?.child?.hintKeys == ["r", "f"])
        #expect(shown?.child?.hintLabel == "this one")
        #expect(table.model.row(at: 1)?.child?.hintKeys == ["r", "f"], "every message child carries them; the cursor decides who draws")
        #expect(table.model.row(at: 3)?.child?.hintKeys == [], "the unsent reply acts on nothing")
        #expect(table.model.row(at: 1)?.accessibilityLabel.contains("Ana Ruiz") == true)
    }
}
