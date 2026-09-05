import AppKit
import PostioFFI

/// Drives an `NSTableView` from a ``MessageRowSource``.
///
/// `NSTableView` asks for a count and then for rows, synchronously, on the
/// main thread, for every visible row on every redraw. That is why the
/// boundary offers `rowCount()` and `rowAt(position:)` rather than a page
/// call that could be awaited: a delegate that *could* wait eventually would,
/// and `PRODUCT.md` §18 says a mailbox is never loaded into memory.
///
/// Deliberately not a `List`. SwiftUI's wraps `NSTableView` anyway and hides
/// the two things this needs — real cell reuse, and explicit scroll-anchor
/// control when new mail arrives at the top.
@MainActor
public final class MessageTableController: NSObject {
    /// Where rows come from.
    public var source: MessageRowSource {
        didSet { tableView?.reloadData() }
    }

    /// The table being driven, once one exists.
    public weak var tableView: NSTableView?

    /// How many times a cell was made rather than reused.
    ///
    /// Kept so reuse can be *asserted* rather than eyeballed: a table that
    /// built a fresh view per row would scroll acceptably in a demo and badly
    /// in a real mailbox, and nothing about the appearance would say so.
    public private(set) var cellsCreated = 0

    /// The identifier every row cell is registered and reused under.
    static let cellIdentifier = NSUserInterfaceItemIdentifier("postio.message.row")

    public init(source: MessageRowSource) {
        self.source = source
        super.init()
    }

    /// What to draw at `position`.
    ///
    /// The whole decision, separated from the drawing so it can be asserted
    /// without AppKit: a row that has not arrived yet is a placeholder, not a
    /// blank, because a blank row and a row that is genuinely empty look
    /// identical and only one of them is worth waiting for.
    public func presentation(at position: UInt32) -> RowPresentation {
        guard let row = source.row(at: position) else { return .placeholder }
        // The mark comes from the model, every redraw, rather than from
        // anything this controller remembers. A cached copy is how a table
        // ends up drawing a selection the engine would not act on -- and the
        // engine's answer is the one an action uses.
        return RowPresentation(
            row: row,
            selected: source.isSelected(row.id),
            // The excerpt, when this is a search hit. A result showing its own
            // first line rather than the text that matched it is a result you
            // have to open to understand.
            snippet: source.snippet(for: row.id)
        )
    }

    /// Called when the row under the cursor changes, with its message.
    ///
    /// The *cursor*, not the selection. `PRODUCT.md` §9 keeps them separate:
    /// moving down the list shows a message without adding it to a
    /// multi-message selection, and a frontend that conflates them makes
    /// shift-click destroy what the user had. The full semantics are their own
    /// work; this is the one signal the reading pane needs.
    public var onCursorChanged: ((Int64?) -> Void)?

    /// Called when the user moves the cursor with the mouse, with its row.
    ///
    /// The *row*, because that is what the boundary moves from: after a click,
    /// `j` has to step from where the user clicked, and an id alone would make
    /// the boundary scan the window to find out where that was.
    public var onCursorRowChanged: ((UInt32?) -> Void)?

    /// The message under the cursor, if the row has arrived.
    public func messageAt(row: Int) -> Int64? {
        guard row >= 0 else { return nil }
        return source.row(at: UInt32(row))?.id
    }

    /// The cell to draw into: `existing` if AppKit handed one back, else a new one.
    ///
    /// Separated from the delegate method so the decision is testable on its
    /// own terms. Whether `NSTableView` actually offers a view back is
    /// Apple's business and needs a real row lifecycle to observe; whether
    /// *this* takes the offer is ours, and is the half that can be got wrong.
    public func cell(reusing existing: NSView?) -> MessageRowCell {
        if let reused = existing as? MessageRowCell {
            return reused
        }
        let made = MessageRowCell()
        made.identifier = Self.cellIdentifier
        cellsCreated += 1
        return made
    }

    /// Show the cursor on `row`, scrolling it into view.
    ///
    /// `NSTableView`'s own selection *is* the cursor here — one row, never
    /// more (`allowsMultipleSelection` is off). The multi-message selection
    /// is Postio's, lives behind the boundary and is drawn per row. Keeping
    /// them apart is `PRODUCT.md` §9, and conflating them is what makes
    /// shift-click destroy what the user had built up.
    ///
    /// Guarded against re-entering the delegate: selecting a row programmatically
    /// fires `tableViewSelectionDidChange`, which would report the move back
    /// to the boundary that just made it.
    public func showCursor(on row: UInt32?) {
        guard let tableView else { return }
        guard let row, Int(row) < tableView.numberOfRows else {
            following = true
            tableView.deselectAll(nil)
            following = false
            return
        }
        following = true
        tableView.selectRowIndexes(IndexSet(integer: Int(row)), byExtendingSelection: false)
        tableView.scrollRowToVisible(Int(row))
        following = false
    }

    /// Whether the selection change now arriving is one we just made.
    private var following = false

    /// Reload exactly the rows a delivered page covers.
    ///
    /// Not `reloadData()`: that would drop the selection and the scroll
    /// position every time a page landed behind the user, which on a fast
    /// scroll is constantly.
    public func pageArrived(page: UInt32, pageSize: UInt32 = 50) {
        guard let tableView else { return }
        let first = Int(page * pageSize)
        let count = Int(pageSize)
        let total = Int(source.rowCount)
        guard first < total else { return }
        let range = first..<min(first + count, total)
        tableView.reloadData(
            forRowIndexes: IndexSet(integersIn: range),
            columnIndexes: IndexSet(integersIn: 0..<max(tableView.numberOfColumns, 1))
        )
    }
}

extension MessageTableController: NSTableViewDataSource {
    public func numberOfRows(in tableView: NSTableView) -> Int {
        // From the engine's count, never from an array we hold.
        Int(source.rowCount)
    }
}

extension MessageTableController: NSTableViewDelegate {
    public func tableViewSelectionDidChange(_ notification: Notification) {
        // Ours, echoing back. Reporting it would tell the boundary about a
        // move the boundary made.
        guard !following else { return }
        guard let table = notification.object as? NSTableView else { return }
        onCursorRowChanged?(table.selectedRow < 0 ? nil : UInt32(table.selectedRow))
        onCursorChanged?(messageAt(row: table.selectedRow))
    }


    public func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let existing = tableView.makeView(withIdentifier: Self.cellIdentifier, owner: self)
        let cell = cell(reusing: existing)
        cell.show(presentation(at: UInt32(row)))
        return cell
    }
}
