import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The results list on the Mac (specs/010-focus-search T072; design §3.4,
/// screens 06 and 07): a view-based `NSTableView` over `ResultsModel`,
/// for the reasons the inbox's is one (009 R10) -- real cell reuse, and
/// nothing measured: every row's height is its group's.
///
/// Group headers are group rows. The table's own selection is off, as the
/// inbox's is: the focus ring is the controller's (`FocusResultsCursor`),
/// drawn by the row view as an accent ring around that row only, and a
/// checked result has the neutral selection tint and a filled box in the
/// gutter. A click is told to the controller (`ResultsModel.point`) and
/// moves nothing here.
@MainActor
public final class ResultsTable: NSObject {
    public let model: ResultsModel
    public let tableView: NSTableView
    public let scrollView: NSScrollView

    /// How many cells were made rather than reused.
    public private(set) var cellsCreated = 0

    static let rowIdentifier = NSUserInterfaceItemIdentifier("postio.results.row")
    static let headerIdentifier = NSUserInterfaceItemIdentifier("postio.results.header")
    static let rowViewIdentifier = NSUserInterfaceItemIdentifier("postio.results.rowview")

    public init(model: ResultsModel) {
        self.model = model
        tableView = NSTableView()
        scrollView = NSScrollView()
        super.init()
        configure()
    }

    private func configure() {
        tableView.headerView = nil
        tableView.style = .plain
        tableView.intercellSpacing = .zero
        tableView.gridStyleMask = []
        tableView.usesAutomaticRowHeights = false
        tableView.selectionHighlightStyle = .none
        tableView.allowsEmptySelection = true
        tableView.allowsMultipleSelection = false
        tableView.allowsTypeSelect = false
        tableView.floatsGroupRows = false
        tableView.focusRingType = .none
        tableView.backgroundColor = .textBackgroundColor
        tableView.dataSource = self
        tableView.delegate = self
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("results"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        tableView.setAccessibilityLabel("Search results")

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor

        tableView.target = self
        tableView.action = #selector(clicked)
        tableView.doubleAction = #selector(doubleClicked)
    }

    @objc private func clicked() {
        model.point(tableRow: tableView.clickedRow)
    }

    @objc private func doubleClicked() {
        model.open(tableRow: tableView.clickedRow)
    }

    // MARK: what the engine says

    /// Draw what one of the model's changes said.
    public func apply(_ change: ResultsModel.Change) {
        switch change {
        case .open, .redraw:
            tableView.reloadData()
            if let row = model.cursorRow, row < tableView.numberOfRows {
                tableView.scrollRowToVisible(row)
            } else {
                tableView.scroll(.zero)
            }
        case let .rows(rows):
            let valid = rows.filteredIndexSet { $0 < tableView.numberOfRows }
            guard !valid.isEmpty else { return }
            tableView.reloadData(forRowIndexes: valid, columnIndexes: IndexSet(integer: 0))
        case let .cursor(previous):
            let rows = [previous, model.cursorRow].compactMap { $0 }.filter { $0 < tableView.numberOfRows }
            for row in rows {
                (tableView.rowView(atRow: row, makeIfNecessary: false) as? ResultRowBackground)?
                    .isCursor = row == model.cursorRow
            }
            if let row = model.cursorRow, row < tableView.numberOfRows {
                // The group's header with its first row, so Top hits is
                // never scrolled away from under the ring.
                if case .header = model.item(at: row - 1) { tableView.scrollRowToVisible(row - 1) }
                tableView.scrollRowToVisible(row)
            }
        case .close:
            tableView.reloadData()
        }
    }

    /// Bring the keyboard to the table: `FocusKeyboardHome` in the results.
    public func takeKeyboard() {
        guard let window = tableView.window else { return }
        window.makeFirstResponder(tableView)
    }
}

extension ResultsTable: NSTableViewDataSource {
    public func numberOfRows(in tableView: NSTableView) -> Int {
        model.count
    }
}

extension ResultsTable: NSTableViewDelegate {
    public func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        CGFloat(model.height(at: row))
    }

    public func tableView(_ tableView: NSTableView, isGroupRow row: Int) -> Bool {
        if case .header = model.item(at: row) { return true }
        return false
    }

    public func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        false
    }

    public func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        (tableView.makeView(withIdentifier: Self.rowViewIdentifier, owner: self) as? ResultRowBackground) ?? {
            let made = ResultRowBackground()
            made.identifier = Self.rowViewIdentifier
            return made
        }()
    }

    public func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        switch model.item(at: row) {
        case let .header(index):
            let group = model.groups.indices.contains(index) ? model.groups[index] : nil
            let view = ResultGroupHeaderView(group: group)
            if let cell = tableView.makeView(withIdentifier: Self.headerIdentifier, owner: self)
                as? NSHostingView<ResultGroupHeaderView>
            {
                cell.rootView = view
                return cell
            }
            let cell = NSHostingView(rootView: view)
            cell.sizingOptions = []
            cell.identifier = Self.headerIdentifier
            cellsCreated += 1
            return cell
        case let .row(position):
            let shown = model.row(at: position)
            let view = ResultRowView(row: shown, selecting: model.selected > 0)
            if let rowView = tableView.rowView(atRow: row, makeIfNecessary: false) as? ResultRowBackground {
                dress(rowView, row: row, shown: shown)
            }
            if let cell = tableView.makeView(withIdentifier: Self.rowIdentifier, owner: self)
                as? NSHostingView<ResultRowView>
            {
                cell.rootView = view
                return cell
            }
            let cell = NSHostingView(rootView: view)
            cell.sizingOptions = []
            cell.identifier = Self.rowIdentifier
            cellsCreated += 1
            return cell
        case nil:
            return nil
        }
    }

    public func tableView(_ tableView: NSTableView, didAdd rowView: NSTableRowView, forRow row: Int) {
        guard let rowView = rowView as? ResultRowBackground else { return }
        if case let .row(position) = model.item(at: row) {
            dress(rowView, row: row, shown: model.row(at: position))
        } else {
            rowView.isHeader = true
            rowView.isCursor = false
            rowView.isChecked = false
        }
    }

    private func dress(_ rowView: ResultRowBackground, row: Int, shown: ResultRowFfi?) {
        rowView.isHeader = false
        rowView.isCursor = row == model.cursorRow
        rowView.isChecked = shown?.checked == true
    }
}

// MARK: - the row view: the ground, the hairline, the ring

/// What sits behind a result's words: the list's surface, a checked
/// result's neutral tint, the hairline under every row, and the focus
/// ring's accent around the focused row.
final class ResultRowBackground: NSTableRowView {
    var isHeader = false { didSet { if isHeader != oldValue { needsDisplay = true } } }
    var isCursor = false { didSet { if isCursor != oldValue { needsDisplay = true } } }
    var isChecked = false { didSet { if isChecked != oldValue { needsDisplay = true } } }

    override var isFlipped: Bool { true }

    override func drawBackground(in dirtyRect: NSRect) {
        NSColor.textBackgroundColor.setFill()
        bounds.fill()
        if isChecked {
            // Neutral, never the accent, which is the ring's.
            NSColor.labelColor.withAlphaComponent(0.06).setFill()
            bounds.fill()
        }
        if isCursor {
            NSColor.controlAccentColor.withAlphaComponent(0.07).setFill()
            bounds.fill()
        }
        NSColor.separatorColor.setFill()
        NSRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1).fill()
    }

    override func drawSelection(in dirtyRect: NSRect) {}
    override func drawSeparator(in dirtyRect: NSRect) {}

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard isCursor else { return }
        let ring = NSBezierPath(rect: bounds.insetBy(dx: 1, dy: 1))
        ring.lineWidth = 2
        NSColor.controlAccentColor.setStroke()
        ring.stroke()
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        isHeader = false
        isCursor = false
        isChecked = false
    }
}

/// The results table in a SwiftUI tree: the main window's results mode.
public struct ResultsTableRepresentable: NSViewRepresentable {
    private let table: ResultsTable

    public init(table: ResultsTable) {
        self.table = table
    }

    public func makeNSView(context: Context) -> NSScrollView {
        table.scrollView
    }

    public func updateNSView(_ scroll: NSScrollView, context: Context) {}
}
