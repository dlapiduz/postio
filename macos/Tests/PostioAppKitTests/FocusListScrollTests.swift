import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// Scrolling a ten-thousand-conversation inbox, counted rather than timed
/// (specs/009-focus-macos T035, constitution V).
///
/// A shared runner cannot defend 16 ms, so what is held is the cause of the
/// budget: the table makes a screenful of row views and cells however far it
/// scrolls, and a frame asks the engine for the rows it draws and a page's
/// worth of read-ahead -- never for the list. Each number below is the same
/// on any machine, and each fails the way the regression would: a table that
/// stopped reusing makes ten thousand views, and one that asked for every
/// row it laid out would ask ten thousand times.
///
/// The source stands in for the session as `focusRow(at:)` answers it:
/// synchronous, `nil` until a page has landed, and a miss is the ask. The
/// engine's own half -- that a miss asks for one page, once, and that the
/// window keeps a bounded number of rows -- is `ffi_suite`'s
/// `focus_scroll.rs`, over a real seeded store.
@MainActor
struct FocusListScrollTests {
    /// Ten thousand rows of which a page is resident only once it has
    /// landed, counting every question the table puts.
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

        private static func row(_ id: Int64, marked: Bool) -> FocusRowFfi {
            FocusRowFfi(
                kind: .conversation, id: id, thread: id, sender: "Ada Moreno",
                subject: "Atlas headcount numbers", preview: "Quick one.", time: "12:20",
                dayHeading: "Today · Saturday 26 September", unread: id % 3 == 0,
                countBadge: nil, hasAttachments: false, sendState: nil, pills: [],
                marker: marked
                    ? MarkerLineFfi(
                        chip: "Question", date: nil, quote: "Do you have the numbers?",
                        status: nil,
                        actions: [FocusRowActionFfi(command: "reply", label: "Reply")])
                    : nil)
        }

        /// The pages missed since the last call have arrived; say which.
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

    /// One frame of scrolling: move to `y`, lay out, and let the pages the
    /// frame missed land the way `FocusPageReady` brings them.
    struct Frame {
        let visible: Int
        /// Rows asked for while laying the frame out.
        let asked: Int
        /// Rows asked for while the pages it missed landed and redrew.
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
        // Down the first stretch a screen at a time, then to the middle, the
        // end, and back to the top: every kind of move a person makes.
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
        // Reuse: views made are the most that were ever on screen, plus the
        // pair AppKit keeps ready, not one per row scrolled past.
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
            // Laying a frame out asks for the rows on screen and the one
            // before the first (whose day decides its heading): the visible
            // rows plus, at most, a page of read-ahead.
            #expect(
                frame.asked <= frame.visible + pageSize,
                "\(frame.asked) rows asked to lay out \(frame.visible) on screen")
            // A page landing is read once, whole, and the rows of it on
            // screen are drawn again: a page plus two screenfuls, not the
            // list.
            #expect(
                frame.askedOnLanding <= pageSize + 2 * frame.visible,
                "\(frame.askedOnLanding) rows asked as \(frame.pagesMissed) page(s) landed")
            // And the pages missed are the ones under the frame -- its own
            // and a neighbour -- not a sweep of the list.
            #expect(frame.pagesMissed <= 2, "\(frame.pagesMissed) pages missed in one frame")
        }
    }

    @Test func layingOutTenThousandRowsAsksForNone() {
        // Heights come from what the model already knows, so building the
        // table over the whole inbox must not fetch a page of it
        // (`FocusListTable.tableView(_:heightOfRow:)`).
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
