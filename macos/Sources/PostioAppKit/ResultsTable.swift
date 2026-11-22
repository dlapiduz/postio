import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// A view-based `NSTableView` for the inbox's reasons (009 R10). Its own selection is off: the
/// focus ring is the controller's and a click is told to it, moving nothing here.
@MainActor
public final class ResultsTable: NSObject {
    public let model: ResultsModel
    public let tableView: NSTableView
    public let scrollView: NSScrollView

    public private(set) var cellsCreated = 0

    static let rowIdentifier = NSUserInterfaceItemIdentifier("postio.results.row")
    static let headerIdentifier = NSUserInterfaceItemIdentifier("postio.results.header")
    static let childIdentifier = NSUserInterfaceItemIdentifier("postio.results.child")
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
        tableView.setAccessibilityLabel(focusSearchWords().resultsLabel)

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor

        tableView.target = self
        tableView.action = #selector(clicked)
        tableView.doubleAction = #selector(doubleClicked)
    }

    public var onContactName: ((Int, String) -> Void)?

    public func nameAnchor(row: Int) -> NSRect? {
        guard row >= 0, row < tableView.numberOfRows else { return nil }
        let frame = tableView.rect(ofRow: row)
        let x = ResultRowView.Columns.leading + ResultRowView.Columns.gutter + ResultRowView.Columns.gap
        return NSRect(x: frame.minX + x, y: frame.minY + 8, width: 120, height: 20)
    }

    public var cursorTableRow: Int? { model.cursorRow }

    @objc private func clicked() {
        model.point(tableRow: tableView.clickedRow)
    }

    @objc private func doubleClicked() {
        model.open(tableRow: tableView.clickedRow)
    }

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
                if case .header = model.item(at: row - 1) { tableView.scrollRowToVisible(row - 1) }
                tableView.scrollRowToVisible(row)
            }
        case let .spliced(removed, inserted):
            let animation: NSTableView.AnimationOptions =
                Motion.duration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion) == 0
                ? [] : .effectFade
            tableView.beginUpdates()
            if let removed { tableView.removeRows(at: IndexSet(integersIn: removed), withAnimation: animation) }
            if let inserted { tableView.insertRows(at: IndexSet(integersIn: inserted), withAnimation: animation) }
            tableView.endUpdates()
        case .close:
            tableView.reloadData()
        }
    }

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
            let view = ResultRowView(
                row: shown, selecting: model.selected > 0,
                onName: { [weak self] address in self?.onContactName?(row, address) })
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
        case let .child(index):
            let view = ResultChildView(child: model.child(at: index))
            if let cell = tableView.makeView(withIdentifier: Self.childIdentifier, owner: self)
                as? NSHostingView<ResultChildView>
            {
                cell.rootView = view
                return cell
            }
            let cell = NSHostingView(rootView: view)
            cell.sizingOptions = []
            cell.identifier = Self.childIdentifier
            cellsCreated += 1
            return cell
        case nil:
            return nil
        }
    }

    public func tableView(_ tableView: NSTableView, didAdd rowView: NSTableRowView, forRow row: Int) {
        guard let rowView = rowView as? ResultRowBackground else { return }
        switch model.item(at: row) {
        case let .row(position):
            dress(rowView, row: row, shown: model.row(at: position))
        case .child:
            dress(rowView, row: row, shown: nil)
            rowView.isChild = true
        default:
            rowView.isHeader = true
            rowView.isCursor = false
            rowView.isChecked = false
            rowView.isChild = false
        }
    }

    private func dress(_ rowView: ResultRowBackground, row: Int, shown: ResultRowFfi?) {
        rowView.isHeader = false
        rowView.isChild = false
        rowView.isCursor = row == model.cursorRow
        rowView.isChecked = shown?.checked == true
    }
}

final class ResultRowBackground: NSTableRowView {
    var isHeader = false { didSet { if isHeader != oldValue { needsDisplay = true } } }
    var isCursor = false { didSet { if isCursor != oldValue { needsDisplay = true } } }
    var isChecked = false { didSet { if isChecked != oldValue { needsDisplay = true } } }
    var isChild = false { didSet { if isChild != oldValue { needsDisplay = true } } }

    override var isFlipped: Bool { true }

    override func drawBackground(in dirtyRect: NSRect) {
        NSColor.textBackgroundColor.setFill()
        bounds.fill()
        if isChild {
            NSColor.labelColor.withAlphaComponent(0.03).setFill()
            bounds.fill()
            NSColor.separatorColor.setFill()
            NSRect(x: ResultRowView.Columns.leading + ResultRowView.Columns.gutter / 2, y: 0, width: 1, height: bounds.height)
                .fill()
        }
        if isChecked {
            NSColor.labelColor.withAlphaComponent(0.06).setFill()
            bounds.fill()
        }
        if isCursor {
            let dark = effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            NSColor.controlAccentColor
                .withAlphaComponent(SearchRuns.focusFill(dark ? .dark : .light))
                .setFill()
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
        isChild = false
    }
}

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
