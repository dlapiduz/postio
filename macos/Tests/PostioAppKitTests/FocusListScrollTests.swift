import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

@MainActor
struct FocusListScrollTests {
    final class TenThousand: FocusRowSource {
        static let total: UInt32 = 10_000
        private let pageSize: UInt32
        private(set) var asked = 0
        private(set) var pagesMissed: Set<UInt32> = []
        private var landed: Set<UInt32> = []

        init(pageSize: UInt32) { self.pageSize = pageSize }

        var focusRowCount: UInt32 { Self.total }

        func focusRow(at position: UInt32) -> FocusRowFfi? {
            asked += 1
            let page = position / pageSize
            guard landed.contains(page) else {
                pagesMissed.insert(page)
                return nil
            }
            return Self.row(Int64(position), marked: position % 9 == 0)
        }

        func residentFocusRow(at position: UInt32) -> FocusRowFfi? {
            landed.contains(position / pageSize) ? Self.row(Int64(position), marked: position % 9 == 0) : nil
        }

        private static func row(_ id: Int64, marked: Bool) -> FocusRowFfi {
            FocusRowFfi(
                kind: .conversation, id: id, thread: id, threads: [id], sender: "Ada Moreno", fromAddress: "ada@example.org",
                subject: "Atlas headcount numbers", preview: "Quick one.", time: "12:20",
                dayHeading: "Today · Saturday 26 September", unread: id % 3 == 0,
                countBadge: nil, hasAttachments: false, sendState: nil, pills: [],
                marker: marked
                    ? MarkerLineFfi(
                        chip: "Question", date: nil, quote: "Do you have the numbers?",
                        status: nil,
                        actions: [FocusRowActionFfi(command: "reply", label: "Reply")])
                    : nil,
                writes: false, threadRow: nil, child: nil)
        }

        func landMissedPages() -> [UInt32] {
            let arrived = pagesMissed.subtracting(landed).sorted()
            landed.formUnion(arrived)
            pagesMissed = []
            return arrived
        }

        func resetCount() {
            asked = 0
            pagesMissed = []
        }
    }

    struct Frame {
        let visible: Int
        let asked: Int
        let askedOnLanding: Int
        let pagesMissed: Int
    }

    static func scroll(_ table: FocusListTable, _ source: TenThousand, to y: CGFloat) -> Frame {
        source.resetCount()
        let clip = table.scrollView.contentView
        clip.scroll(to: NSPoint(x: 0, y: y))
        table.scrollView.reflectScrolledClipView(clip)
        table.tableView.layoutSubtreeIfNeeded()
        let askedByLayout = source.asked
        let missed = source.landMissedPages()
        source.resetCount()
        for page in missed { table.pageArrived(page) }
        table.tableView.layoutSubtreeIfNeeded()
        let visible = table.tableView.rows(in: table.tableView.visibleRect).length
        return Frame(
            visible: visible, asked: askedByLayout, askedOnLanding: source.asked,
            pagesMissed: missed.count)
    }

    static func tenThousand() -> (FocusListTable, TenThousand) {
        let source = TenThousand(pageSize: FocusListModel.pageSize)
        let model = FocusListModel(source: source) { $0 == "reply" ? "e" : nil }
        let table = FocusListTable(model: model)
        table.scrollView.frame = NSRect(x: 0, y: 0, width: 1440, height: 800)
        table.listChanged(total: TenThousand.total)
        table.tableView.layoutSubtreeIfNeeded()
        return (table, source)
    }

    @Test func aScreenfulOfViewsServesAnyDistanceOfScrolling() {
        let (table, source) = Self.tenThousand()
        let page = table.scrollView.contentView.bounds.height
        var frames: [Frame] = []
        var y: CGFloat = 0
        for _ in 0..<60 {
            frames.append(Self.scroll(table, source, to: y))
            y += page
        }
        for target in [0.5, 1.0, 0.0] {
            let end = table.tableView.frame.height - page
            frames.append(Self.scroll(table, source, to: max(0, end * target)))
        }

        let widest = frames.map(\.visible).max() ?? 0
        #expect(widest > 0, "nothing was laid out, so this proves nothing")
        #expect(
            table.rowViewsCreated <= widest + 8,
            "\(table.rowViewsCreated) row views for at most \(widest) on screen")
        #expect(
            table.cellsCreated <= widest + 8,
            "\(table.cellsCreated) cells for at most \(widest) on screen")
        #expect(
            table.rowViewsCreated < frames.count,
            "a view per frame is a view per page of scrolling")
    }

    @Test func aFrameAsksForTheRowsItDrawsAndOnePageOfReadAhead() {
        let (table, source) = Self.tenThousand()
        let page = table.scrollView.contentView.bounds.height
        let pageSize = Int(FocusListModel.pageSize)
        var y: CGFloat = 0
        for _ in 0..<60 {
            let frame = Self.scroll(table, source, to: y)
            y += page
            #expect(
                frame.asked <= frame.visible + pageSize,
                "\(frame.asked) rows asked to lay out \(frame.visible) on screen")
            #expect(
                frame.askedOnLanding <= pageSize + 2 * frame.visible,
                "\(frame.askedOnLanding) rows asked as \(frame.pagesMissed) page(s) landed")
            #expect(frame.pagesMissed <= 2, "\(frame.pagesMissed) pages missed in one frame")
        }
    }

    @Test func layingOutTenThousandRowsAsksForNone() {
        // Heights come from what the model knows: building the table over the whole inbox must not
        // fetch a page.
        let source = TenThousand(pageSize: FocusListModel.pageSize)
        let model = FocusListModel(source: source) { _ in nil }
        let table = FocusListTable(model: model)
        table.scrollView.frame = NSRect(x: 0, y: 0, width: 1440, height: 800)
        table.listChanged(total: TenThousand.total)
        table.tableView.layoutSubtreeIfNeeded()
        let visible = table.tableView.rows(in: table.tableView.visibleRect).length
        #expect(
            source.asked <= visible + Int(FocusListModel.pageSize),
            "\(source.asked) rows asked to lay out \(visible) visible rows of \(TenThousand.total)")
    }
}
