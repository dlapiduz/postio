import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// An `NSTableView`, not a SwiftUI `List` (R10): ten thousand rows at 60 fps with real cell reuse.
/// Row heights come from what the model already knows, so layout never fetches a page.
/// The table's own selection is off: the cursor and selection are the controller's and arrive as
/// intents; `NSTableView`'s would be a second opinion and a blue fill the design does not draw.
@MainActor
public final class FocusListTable: NSObject {
    public let model: FocusListModel
    public let tableView: NSTableView
    public let scrollView: NSScrollView

    public var onAction: ((String, Int) -> Void)?

    /// The ring moves when the controller's `FocusCursor` comes back, never here.
    public var onPoint: ((Int) -> Void)?
    public var onActivate: ((Int) -> Void)?

    public var onChevron: ((Int) -> Void)?

    public var onPick: ((Int, Bool) -> Void)?

    /// An undo that brings rows in above keeps the list there only if it was at the top.
    public var onAtTop: ((Bool) -> Void)?

    private var saidAtTop: Bool?

    /// So reuse can be asserted rather than eyeballed.
    public private(set) var cellsCreated = 0

    public private(set) var rowViewsCreated = 0

    static let cellIdentifier = NSUserInterfaceItemIdentifier("postio.focus.row")
    static let rowIdentifier = NSUserInterfaceItemIdentifier("postio.focus.rowview")

    private var heightsOwed = IndexSet()

    public init(model: FocusListModel) {
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
        tableView.focusRingType = .none
        tableView.backgroundColor = .textBackgroundColor
        tableView.dataSource = self
        tableView.delegate = self
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("focus"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        tableView.setAccessibilityLabel("Inbox")

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor

        tableView.target = self
        tableView.action = #selector(rowClicked)
        tableView.doubleAction = #selector(rowDoubleClicked)

        scrollView.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(boundsChanged),
            name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
    }

    @objc private func rowClicked() {
        let row = tableView.clickedRow
        let event = NSApp.currentEvent
        if let event, event.modifierFlags.intersection([.shift, .command, .option, .control]).isEmpty,
            let address = model.row(at: row)?.fromAddress,
            let anchor = nameAnchor(row: row),
            anchor.contains(tableView.convert(event.locationInWindow, from: nil))
        {
            onPoint?(row)
            onContactName?(row, address)
            return
        }
        if let event, event.modifierFlags.intersection([.shift, .command, .option, .control]).isEmpty,
            row >= 0, model.row(at: row)?.chevron != nil,
            Self.chevronHit(tableView.convert(event.locationInWindow, from: nil).x)
        {
            onChevron?(row)
            return
        }
        clicked(row: row, modifiers: event?.modifierFlags ?? [])
    }

    static func chevronHit(_ x: CGFloat) -> Bool {
        abs(x - CGFloat(FocusRowMetrics.chevronCentre)) <= 10
    }

    public var onContactName: ((Int, String) -> Void)?

    public func nameAnchor(row: Int) -> NSRect? {
        guard row >= 0, row < tableView.numberOfRows, let shown = model.row(at: row),
            shown.fromAddress != nil
        else { return nil }
        let frame = tableView.rect(ofRow: row)
        let band = shown.heading == nil ? 0 : CGFloat(FocusRowMetrics.heading)
        let columns = FocusRowMetrics.columns(width: Double(frame.width))
        let bold = shown.bold || shown.layout == .digest
        let name = Ink.width(shown.sender, bold ? Ink.bodyBold : Ink.body)
        let width = min(name, CGFloat(columns.senderWidth))
        let middle = frame.minY + band + CGFloat(FocusRowMetrics.firstLine)
        return NSRect(x: frame.minX + CGFloat(FocusRowMetrics.senderX), y: middle - 10, width: width, height: 20)
    }

    @objc private func rowDoubleClicked() {
        activated(row: tableView.clickedRow)
    }

    public func activated(row: Int) {
        guard row >= 0, row < model.count else { return }
        onActivate?(row)
    }

    @objc private func boundsChanged() {
        scrolled()
    }

    public func listChanged(total: UInt32) {
        model.reset(total: total)
        heightsOwed = []
        tableView.reloadData()
        if let cursor = model.cursor { tableView.scrollRowToVisible(cursor) }
        scrolled()
    }

    /// Inserted, not reloaded, so only what is below moves. After a re-read with a thread open the
    /// rows on screen were drawn by the session's splice, so the list is drawn again whole rather
    /// than slid.
    public func splice(anchor: Int, children: Int) {
        let folded = model.open
        let reread = model.rereadWhileOpen
        let inserted = model.spliced(anchor: anchor, children: children)
        heightsOwed = []
        if reread {
            tableView.reloadData()
            return
        }
        changing { [tableView] in
            if let folded {
                tableView.removeRows(at: IndexSet(folded.range), withAnimation: [])
            }
            tableView.insertRows(at: IndexSet(integersIn: inserted), withAnimation: Self.entrance)
        }
        redraw(rows: [anchor])
    }

    public func unsplice() {
        let reread = model.rereadWhileOpen
        guard let anchor = model.open?.anchor, let gone = model.unspliced() else { return }
        heightsOwed = []
        if reread {
            tableView.reloadData()
            return
        }
        changing { [tableView] in
            tableView.removeRows(at: IndexSet(integersIn: gone), withAnimation: Self.exit)
        }
        redraw(rows: [anchor])
    }

    private static var entrance: NSTableView.AnimationOptions {
        Motion.duration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion) == 0
            ? [] : .effectFade
    }
    private static var exit: NSTableView.AnimationOptions { entrance }

    private func changing(_ edit: () -> Void) {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = Motion.duration(
                reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
            context.allowsImplicitAnimation = context.duration > 0
            tableView.beginUpdates()
            edit()
            tableView.endUpdates()
        }
    }

    /// Never `reloadData()`: a page landing behind the reader must not move the list they are
    /// looking at.
    public func pageArrived(_ page: UInt32) {
        let changed = model.pageArrived(page)
        guard !changed.isEmpty else { return }
        noteHeights(changed)
        tableView.reloadData(forRowIndexes: changed, columnIndexes: IndexSet(integer: 0))
    }

    public func keymapChanged() {
        model.keymapChanged()
        redrawVisible()
    }

    public var cursor: Int? { model.cursor }

    public func pickerAnchor(row: Int, width: CGFloat) -> NSRect? {
        guard row >= 0, row < tableView.numberOfRows else { return nil }
        let frame = tableView.rect(ofRow: row)
        let band = model.row(at: row)?.heading == nil ? 0 : CGFloat(FocusRowMetrics.heading)
        let subject = CGFloat(FocusRowMetrics.columns(width: Double(frame.width)).subjectX)
        return NSRect(x: frame.minX + subject, y: frame.minY + band, width: width, height: frame.height - band)
    }

    public var selected: Set<Int64> { model.selected }

    public func clicked(row: Int, modifiers: NSEvent.ModifierFlags) {
        guard row >= 0, row < model.count else { return }
        if modifiers.contains(.shift) {
            onPick?(row, true)
        } else if modifiers.contains(.command) {
            onPick?(row, false)
        } else {
            onPoint?(row)
        }
    }

    public func apply(_ change: FocusIntents.Change) {
        switch change {
        case let .cursor(previous, toTop):
            redraw(rows: [previous, model.cursor].compactMap { $0 })
            if toTop {
                scrollToTop()
            } else if let row = model.cursor, row < tableView.numberOfRows {
                tableView.scrollRowToVisible(row)
            }
        case .selection:
            redrawVisible()
        case .heading:
            noteHeights(IndexSet(integersIn: 0..<tableView.numberOfRows))
            redrawVisible()
        case .listToTop:
            scrollToTop()
        case .toast:
            break
        }
    }

    private func scrollToTop() {
        scrollView.contentView.scroll(to: .zero)
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    func scrolled() {
        guard let onAtTop else { return }
        let atTop = scrollView.contentView.bounds.origin.y <= 0
        guard atTop != saidAtTop else { return }
        saidAtTop = atTop
        onAtTop(atTop)
    }

    private func redraw(rows: [Int]) {
        let valid = IndexSet(rows.filter { $0 >= 0 && $0 < tableView.numberOfRows })
        guard !valid.isEmpty else { return }
        tableView.reloadData(forRowIndexes: valid, columnIndexes: IndexSet(integer: 0))
    }

    private func redrawVisible() {
        let visible = tableView.rows(in: tableView.visibleRect)
        guard visible.length > 0 else { return }
        redraw(rows: Array(visible.location..<(visible.location + visible.length)))
    }

    /// Changing heights from inside `viewFor` re-enters the layout that asked.
    private func oweHeight(_ row: Int) {
        let wasEmpty = heightsOwed.isEmpty
        heightsOwed.insert(row)
        guard wasEmpty else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let owed = self.heightsOwed.filteredIndexSet { $0 < self.tableView.numberOfRows }
            self.heightsOwed = []
            guard !owed.isEmpty else { return }
            self.noteHeights(owed)
        }
    }

    /// `noteHeightOfRows` animates by default, so a page landing after a delete re-read slid every
    /// row into place and the list appeared to collapse and grow back.
    private func noteHeights(_ rows: IndexSet) {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            context.allowsImplicitAnimation = false
            tableView.noteHeightOfRows(withIndexesChanged: rows)
        }
    }

    func cell(reusing existing: NSView?) -> FocusRowCell {
        if let reused = existing as? FocusRowCell { return reused }
        let made = FocusRowCell()
        made.identifier = Self.cellIdentifier
        cellsCreated += 1
        return made
    }
}

extension FocusListTable: NSTableViewDataSource {
    public func numberOfRows(in tableView: NSTableView) -> Int {
        model.count
    }
}

extension FocusListTable: NSTableViewDelegate {
    public func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        // From the model's memory of the row's shape, never the FFI, which would fetch every page
        // of a 10k inbox.
        CGFloat(model.height(at: row))
    }

    public func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        false
    }

    public func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let view = (tableView.makeView(withIdentifier: Self.rowIdentifier, owner: self)
            as? FocusRowView) ?? {
                let made = FocusRowView()
                made.identifier = Self.rowIdentifier
                rowViewsCreated += 1
                return made
            }()
        return view
    }

    public func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let cell = cell(reusing: tableView.makeView(withIdentifier: Self.cellIdentifier, owner: self))
        let shown = model.row(at: row)
        cell.show(
            shown, picked: shown.map { model.isPicked($0.id) } ?? false,
            cursor: row == model.cursor)
        cell.onAction = { [weak self] command in self?.onAction?(command, row) }
        if let rowView = tableView.rowView(atRow: row, makeIfNecessary: false) as? FocusRowView {
            dress(rowView, row: row, shown: shown)
        }
        if CGFloat(model.height(at: row)) != tableView.rect(ofRow: row).height {
            oweHeight(row)
        }
        return cell
    }

    public func tableView(_ tableView: NSTableView, didAdd rowView: NSTableRowView, forRow row: Int) {
        guard let rowView = rowView as? FocusRowView else { return }
        dress(rowView, row: row, shown: (rowView.view(atColumn: 0) as? FocusRowCell)?.shown)
    }

    private func dress(_ rowView: FocusRowView, row: Int, shown: FocusRowModel?) {
        rowView.isCursor = row == model.cursor
        rowView.heading = shown?.heading != nil
        rowView.isPicked = shown.map { model.isPicked($0.id) } ?? false
        if let child = shown?.child, let open = model.open {
            rowView.child = FocusRowView.ChildMark(unread: child.unread, last: row == open.range.upperBound - 1)
        } else {
            rowView.child = nil
        }
    }
}

final class FocusRowView: NSTableRowView {
    var heading = false { didSet { if heading != oldValue { needsDisplay = true } } }
    var isCursor = false { didSet { if isCursor != oldValue { needsDisplay = true } } }
    var isPicked = false { didSet { if isPicked != oldValue { needsDisplay = true } } }

    struct ChildMark: Equatable {
        let unread: Bool
        let last: Bool
    }
    var child: ChildMark? { didSet { if child != oldValue { needsDisplay = true } } }

    override var isFlipped: Bool { true }

    var body: NSRect {
        let top = heading ? CGFloat(FocusRowMetrics.heading) : 0
        return NSRect(x: bounds.minX, y: top, width: bounds.width, height: bounds.height - top)
    }

    override func drawBackground(in dirtyRect: NSRect) {
        NSColor.textBackgroundColor.setFill()
        bounds.fill()
        if heading {
            NSColor.windowBackgroundColor.setFill()
            NSRect(x: 0, y: 0, width: bounds.width, height: CGFloat(FocusRowMetrics.heading)).fill()
            NSColor.separatorColor.setFill()
            NSRect(x: 0, y: CGFloat(FocusRowMetrics.heading) - 1, width: bounds.width, height: 1).fill()
        }
        if isPicked {
            // A neutral ground, never the accent, which is the cursor's. `withAlphaComponent`
            // replaces the label colour's own alpha.
            NSColor.labelColor.withAlphaComponent(0.06).setFill()
            body.fill()
        }
        if let child {
            NSColor.labelColor.withAlphaComponent(0.03).setFill()
            bounds.fill()
            let line = CGFloat(FocusRowMetrics.childLineX)
            let width = CGFloat(FocusRowMetrics.childLineWidth)
            NSColor.separatorColor.setFill()
            NSRect(
                x: line - width / 2, y: 0, width: width,
                height: child.last ? bounds.midY : bounds.height
            ).fill()
            (child.unread ? NSColor.controlAccentColor : NSColor.tertiaryLabelColor).setFill()
            NSBezierPath(ovalIn: NSRect(x: line - 3.5, y: bounds.midY - 3.5, width: 7, height: 7)).fill()
        } else {
            NSColor.separatorColor.setFill()
            NSRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1).fill()
        }
        if isCursor {
            NSColor.controlAccentColor.withAlphaComponent(0.07).setFill()
            body.fill()
        }
    }

    override func drawSelection(in dirtyRect: NSRect) {}
    override func drawSeparator(in dirtyRect: NSRect) {}

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard isCursor else { return }
        let ring = NSBezierPath(rect: body.insetBy(dx: 1, dy: 1))
        ring.lineWidth = 2
        NSColor.controlAccentColor.setStroke()
        ring.stroke()
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        heading = false
        isCursor = false
        isPicked = false
        child = nil
    }
}

final class FocusRowCell: NSView {
    private(set) var shown: FocusRowModel?
    private var picked = false
    var onAction: ((String) -> Void)?

    private let gutterIcon = NSImageView()
    private let chevron = NSImageView()
    private let clip = NSImageView()
    private var cursor = false
    private var buttons: [FocusActionButton] = []

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        for image in [gutterIcon, chevron, clip] {
            image.imageScaling = .scaleProportionallyDown
            image.isHidden = true
            addSubview(image)
        }
        clip.image = NSImage(systemSymbolName: "paperclip", accessibilityDescription: "Attachment")
        clip.contentTintColor = .secondaryLabelColor
        chevron.contentTintColor = .secondaryLabelColor
        setAccessibilityElement(true)
        setAccessibilityRole(.row)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    func show(_ row: FocusRowModel?, picked: Bool, cursor: Bool = false) {
        shown = row
        self.picked = picked
        self.cursor = cursor
        let actions = row?.marker?.actions ?? []
        while buttons.count < actions.count {
            let button = FocusActionButton()
            addSubview(button)
            buttons.append(button)
        }
        for (index, button) in buttons.enumerated() {
            if index < actions.count {
                let action = actions[index]
                button.configure(label: action.label, cap: action.cap, outlined: index == 0)
                button.run = { [weak self] in self?.onAction?(action.command) }
                button.isHidden = false
            } else {
                button.isHidden = true
                button.run = nil
            }
        }
        setAccessibilityLabel(row?.accessibilityLabel ?? "Loading")
        needsLayout = true
        needsDisplay = true
    }

    private var top: CGFloat { shown?.heading != nil ? CGFloat(FocusRowMetrics.heading) : 0 }

    private var firstLine: CGFloat {
        switch shown?.layout {
        case .twoLine: return top + CGFloat(FocusRowMetrics.firstLine)
        case .thread: return top + CGFloat(FocusRowMetrics.thread) / 2
        case .child: return CGFloat(FocusRowMetrics.child) / 2
        default: return top + CGFloat(FocusRowMetrics.oneLine) / 2
        }
    }

    private var secondLine: CGFloat { top + CGFloat(FocusRowMetrics.secondLine) }

    override func layout() {
        super.layout()
        let width = bounds.width
        if let shown, shown.layout != .child, picked || shown.layout == .digest {
            gutterIcon.image = NSImage(
                systemSymbolName: picked ? "checkmark.square.fill" : "square.stack.3d.up",
                accessibilityDescription: nil
            )
            gutterIcon.contentTintColor = picked ? .labelColor : .secondaryLabelColor
            let centre = shown.layout == .digest ? FocusRowMetrics.gutterCentre : FocusRowMetrics.dotCentre
            gutterIcon.frame = NSRect(x: CGFloat(centre) - 7, y: firstLine - 7, width: 14, height: 14)
            gutterIcon.isHidden = false
        } else {
            gutterIcon.isHidden = true
        }
        if let shown, let state = shown.chevron {
            // Two symbols rather than a turn, so Reduce Motion has nothing to turn.
            chevron.image = NSImage(
                systemSymbolName: state == .expanded ? "chevron.down" : "chevron.right",
                accessibilityDescription: state == .expanded ? "Expanded" : "Collapsed"
            )?.withSymbolConfiguration(.init(pointSize: 10, weight: .semibold))
            chevron.frame = NSRect(
                x: CGFloat(FocusRowMetrics.chevronCentre) - 6, y: firstLine - 6, width: 12, height: 12)
            chevron.isHidden = false
        } else {
            chevron.isHidden = true
        }
        if let shown, shown.hasAttachments, shown.layout != .child {
            let at = trailingColumn(of: shown, width: width).clip ?? 0
            clip.frame = NSRect(x: at, y: firstLine - 8, width: 16, height: 16)
            clip.isHidden = false
        } else {
            clip.isHidden = true
        }
        var right = width - CGFloat(FocusRowMetrics.trailing)
        for button in buttons.reversed() where !button.isHidden {
            let size = button.intrinsicContentSize
            right -= size.width
            button.frame = NSRect(
                x: right, y: secondLine - size.height / 2, width: size.width, height: size.height)
            right -= 4
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let row = shown else { return }
        let width = bounds.width
        if let heading = row.heading {
            Ink.draw(
                heading, Ink.heading, colour: .secondaryLabelColor,
                x: 24, middle: CGFloat(FocusRowMetrics.heading) / 2, room: width - 48)
        }
        if let child = row.child {
            drawChild(row, child, width: width)
            return
        }
        let middle = firstLine
        let bold = row.bold || row.layout == .digest

        if row.dot, !picked {
            NSColor.controlAccentColor.setFill()
            NSBezierPath(ovalIn: NSRect(
                x: CGFloat(FocusRowMetrics.dotCentre) - 3.5, y: middle - 3.5, width: 7, height: 7
            )).fill()
        }

        let trailing = trailingColumn(of: row, width: width)
        Ink.draw(row.time, trailing.timeFont, colour: .labelColor, x: trailing.time, middle: middle, room: width)
        if let badge = row.countBadge, let frame = trailing.badge {
            drawCount(badge, in: frame, middle: middle)
        }
        if let new = row.newBadge, let frame = trailing.newPill {
            // Label colour, never the accent (reserved for focus and action).
            NSColor.labelColor.setFill()
            NSBezierPath(roundedRect: frame, xRadius: frame.height / 2, yRadius: frame.height / 2).fill()
            Ink.draw(
                new, Ink.newPill, colour: .textBackgroundColor, x: frame.minX + 8, middle: middle,
                room: frame.width - 8)
        }
        if let muted = row.mutedBadge, let at = trailing.muted {
            Ink.draw(muted, Ink.body, colour: .secondaryLabelColor, x: at, middle: middle, room: width)
        }
        if let state = row.sendState, let at = trailing.sendState {
            Ink.draw(state, Ink.body, colour: .secondaryLabelColor, x: at, middle: middle, room: width)
        }
        let end = trailing.end

        let columns = FocusRowMetrics.columns(width: Double(width))
        let x0 = CGFloat(FocusRowMetrics.senderX)
        if row.chevron != nil, let count = row.countBadge {
            let boxed = Ink.width(count, Ink.badge) + 10
            let room = min(CGFloat(columns.senderWidth), end - x0) - boxed - 6
            let drawn = Ink.draw(
                row.sender, bold ? Ink.bodyBold : Ink.body, colour: .labelColor,
                x: x0, middle: middle, room: room)
            drawCount(
                count, in: NSRect(x: x0 + drawn + 6, y: middle - 8, width: boxed, height: 16), middle: middle)
        } else {
            Ink.draw(
                row.sender, bold ? Ink.bodyBold : Ink.body, colour: .labelColor,
                x: x0, middle: middle, room: min(CGFloat(columns.senderWidth), end - x0))
        }
        var x = CGFloat(columns.subjectX)
        x += Ink.draw(
            row.subject, bold ? Ink.bodyBold : Ink.body, colour: .labelColor,
            x: x, middle: middle, room: end - x)

        for pill in row.pills {
            let name = Ink.width(pill.name, Ink.pill)
            let boxed = 7 + 6 + 5 + name + 7
            guard x + CGFloat(FocusRowMetrics.gap) + boxed <= end else { break }
            x += CGFloat(FocusRowMetrics.gap)
            let frame = NSRect(x: x, y: middle - 10, width: boxed, height: 20)
            NSColor.separatorColor.setStroke()
            let outline = NSBezierPath(roundedRect: frame.insetBy(dx: 0.5, dy: 0.5), xRadius: 10, yRadius: 10)
            outline.lineWidth = 1
            outline.stroke()
            LabelDot.colour(pill.colour).setFill()
            NSBezierPath(ovalIn: NSRect(x: x + 7, y: middle - 3, width: 6, height: 6)).fill()
            Ink.draw(pill.name, Ink.pill, colour: .secondaryLabelColor, x: x + 18, middle: middle, room: name)
            x += boxed
        }

        if let preview = row.preview, end - (x + CGFloat(FocusRowMetrics.gap)) >= 40 {
            x += CGFloat(FocusRowMetrics.gap)
            Ink.draw(preview, Ink.body, colour: .secondaryLabelColor, x: x, middle: middle, room: end - x)
        }

        if let marker = row.marker { drawMarker(marker, width: width, x: CGFloat(columns.markerX)) }
    }

    private func drawCount(_ text: String, in frame: NSRect, middle: CGFloat) {
        NSColor.separatorColor.setStroke()
        let outline = NSBezierPath(roundedRect: frame.insetBy(dx: 0.5, dy: 0.5), xRadius: 4, yRadius: 4)
        outline.lineWidth = 1
        outline.stroke()
        Ink.draw(text, Ink.badge, colour: .secondaryLabelColor, x: frame.minX + 5, middle: middle, room: frame.width)
    }

    private func drawChild(_ row: FocusRowModel, _ child: FocusRowModel.Child, width: CGFloat) {
        let middle = firstLine
        let gap = CGFloat(FocusRowMetrics.gap)
        let bold = child.unread

        let timeFont = bold ? Ink.timeBold : Ink.time
        var x = width - CGFloat(FocusRowMetrics.trailing) - Ink.width(row.time, timeFont)
        Ink.draw(row.time, timeFont, colour: child.draft ? .secondaryLabelColor : .labelColor, x: x, middle: middle, room: width)
        if child.attachment {
            x -= gap + 16
            clip.frame = NSRect(x: x, y: middle - 8, width: 16, height: 16)
            clip.isHidden = false
        }
        if cursor, let label = child.hintLabel, !child.hintKeys.isEmpty {
            x -= gap + Ink.width(label, Ink.hint)
            Ink.draw(label, Ink.hint, colour: .secondaryLabelColor, x: x, middle: middle, room: width)
            for key in child.hintKeys.reversed() {
                x -= 4 + KeyCapView.width(of: key)
                KeyCapView.draw(key, x: x, middle: middle)
            }
        }
        let end = x - gap

        let diameter = CGFloat(FocusRowMetrics.childAvatar)
        let avatar = NSRect(x: CGFloat(FocusRowMetrics.childAvatarX), y: middle - diameter / 2, width: diameter, height: diameter)
        NSColor.tertiaryLabelColor.setFill()
        NSBezierPath(ovalIn: avatar).fill()
        let letters = Ink.width(child.initials, Ink.avatar)
        Ink.draw(
            child.initials, Ink.avatar, colour: .labelColor, x: avatar.midX - letters / 2, middle: middle,
            room: diameter)
        let senderX = avatar.maxX + 7
        let columns = FocusRowMetrics.columns(width: Double(width))
        let subjectX = CGFloat(columns.subjectX)
        let senderFont = child.draft ? Ink.bodyItalic : (bold ? Ink.bodyBold : Ink.body)
        Ink.draw(child.sender, senderFont, colour: .labelColor, x: senderX, middle: middle, room: subjectX - gap - senderX)

        var lineX = subjectX
        if let tag = child.tag, !child.draft {
            let boxed = Ink.width(tag, Ink.pill) + 12
            let frame = NSRect(x: lineX, y: middle - 9, width: boxed, height: 18)
            NSColor.quaternaryLabelColor.setFill()
            NSBezierPath(roundedRect: frame, xRadius: 4, yRadius: 4).fill()
            Ink.draw(tag, Ink.pill, colour: .secondaryLabelColor, x: frame.minX + 6, middle: middle, room: boxed)
            lineX += boxed + 8
        }
        var words = child.line
        var font = Ink.body
        var colour = NSColor.secondaryLabelColor
        if child.draft {
            words = "\u{201c}\(child.line)\u{201d}" + (child.tag.map { " \u{b7} \($0)" } ?? "")
            font = Ink.quote
        } else if child.placeholder {
            colour = .tertiaryLabelColor
            font = Ink.quote
        }
        Ink.draw(words, font, colour: colour, x: lineX, middle: middle, room: end - lineX)
    }

    private func trailingColumn(of row: FocusRowModel, width: CGFloat) -> (
        timeFont: NSFont, time: CGFloat, badge: NSRect?, newPill: NSRect?, muted: CGFloat?,
        sendState: CGFloat?, clip: CGFloat?, end: CGFloat
    ) {
        let gap = CGFloat(FocusRowMetrics.gap)
        let timeFont = row.bold || row.layout == .digest ? Ink.timeBold : Ink.time
        var x = width - CGFloat(FocusRowMetrics.trailing) - Ink.width(row.time, timeFont)
        let time = x
        var badge: NSRect?
        if let count = row.countBadge, row.chevron == nil {
            let boxed = Ink.width(count, Ink.badge) + 10
            x -= gap + boxed
            badge = NSRect(x: x, y: firstLine - 8, width: boxed, height: 16)
        }
        var sendState: CGFloat?
        if let state = row.sendState {
            x -= gap + Ink.width(state, Ink.body)
            sendState = x
        }
        var clip: CGFloat?
        if row.hasAttachments {
            x -= gap + 16
            clip = x
        }
        var muted: CGFloat?
        if let badge = row.mutedBadge {
            x -= gap + Ink.width(badge, Ink.body)
            muted = x
        }
        var newPill: NSRect?
        if let new = row.newBadge {
            let boxed = Ink.width(new, Ink.newPill) + 16
            x -= gap + boxed
            newPill = NSRect(x: x, y: firstLine - 9, width: boxed, height: 18)
        }
        return (timeFont, time, badge, newPill, muted, sendState, clip, x - gap)
    }

    private func drawMarker(_ marker: FocusRowModel.Marker, width: CGFloat, x start: CGFloat) {
        let line = secondLine
        var right = width - CGFloat(FocusRowMetrics.trailing)
        for button in buttons where !button.isHidden { right = min(right, button.frame.minX) }
        if let status = marker.status {
            right -= Ink.width(status, Ink.body)
            Ink.draw(status, Ink.body, colour: .secondaryLabelColor, x: right, middle: line, room: width)
        }
        right -= CGFloat(FocusRowMetrics.gap)

        var x = start
        let chip = Ink.width(marker.chip, Ink.chip) + 12
        let frame = NSRect(x: x, y: line - 9, width: chip, height: 18)
        NSColor.controlAccentColor.setStroke()
        let outline = NSBezierPath(roundedRect: frame.insetBy(dx: 0.5, dy: 0.5), xRadius: 4, yRadius: 4)
        outline.lineWidth = 1
        outline.stroke()
        Ink.draw(marker.chip, Ink.chip, colour: .controlAccentColor, x: x + 6, middle: line, room: chip)
        x += chip + 10
        if let date = marker.date {
            x += Ink.draw(date, Ink.date, colour: .controlAccentColor, x: x, middle: line, room: max(0, right - x)) + 10
        }
        if let quoted = marker.quoted, right - x >= 40 {
            Ink.draw(quoted, Ink.quote, colour: .controlAccentColor, x: x, middle: line, room: right - x)
        }
    }

    override func accessibilityChildren() -> [Any]? {
        buttons.filter { !$0.isHidden }
    }
}

final class FocusActionButton: NSView {
    private var label = ""
    private var cap: String?
    private var outlined = false
    var run: (() -> Void)?

    override var isFlipped: Bool { true }

    func configure(label: String, cap: String?, outlined: Bool) {
        self.label = label
        self.cap = cap
        self.outlined = outlined
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel(cap.map { "\(label), key \($0)" } ?? label)
        invalidateIntrinsicContentSize()
        needsDisplay = true
    }

    override var intrinsicContentSize: NSSize {
        let words = Ink.width(label, Ink.action)
        let key = cap.map { 6 + KeyCapView.width(of: $0) } ?? 0
        return NSSize(width: 9 + words + key + 9, height: 24)
    }

    override func draw(_ dirtyRect: NSRect) {
        if outlined {
            let frame = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 5, yRadius: 5)
            NSColor.controlBackgroundColor.setFill()
            frame.fill()
            NSColor.separatorColor.setStroke()
            frame.lineWidth = 1
            frame.stroke()
        }
        let middle = bounds.midY
        let words = Ink.draw(label, Ink.action, colour: .labelColor, x: 9, middle: middle, room: bounds.width)
        if let cap { KeyCapView.draw(cap, x: 9 + words + 6, middle: middle) }
    }

    override func mouseDown(with event: NSEvent) {
        // Taken here so a click on an answer never reaches the table as a click on the row.
    }

    override func mouseUp(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        if bounds.contains(point) { run?() }
    }

    override func accessibilityPerformPress() -> Bool {
        run?()
        return run != nil
    }
}

public final class KeyCapView: NSView {
    public var text: String {
        didSet {
            invalidateIntrinsicContentSize()
            needsDisplay = true
        }
    }

    public init(_ text: String) {
        self.text = text
        super.init(frame: .zero)
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    public override var isFlipped: Bool { true }

    public override var intrinsicContentSize: NSSize {
        NSSize(width: Self.width(of: text), height: KeyCapMetrics.height)
    }

    public override func draw(_ dirtyRect: NSRect) {
        Self.draw(text, x: 0, middle: bounds.midY)
    }

    public static func width(of text: String) -> CGFloat {
        max(KeyCapMetrics.minWidth, Ink.width(text, Ink.keycap) + 2 * KeyCapMetrics.padding)
    }

    public static func draw(_ text: String, x: CGFloat, middle: CGFloat) {
        let width = width(of: text)
        let frame = NSRect(x: x, y: middle - KeyCapMetrics.height / 2, width: width, height: KeyCapMetrics.height)
        let path = NSBezierPath(
            roundedRect: frame.insetBy(dx: KeyCapMetrics.outline / 2, dy: KeyCapMetrics.outline / 2),
            xRadius: KeyCapMetrics.radius, yRadius: KeyCapMetrics.radius)
        NSColor.quaternaryLabelColor.setFill()
        path.fill()
        NSColor.separatorColor.setStroke()
        path.lineWidth = KeyCapMetrics.outline
        path.stroke()
        let words = Ink.width(text, Ink.keycap)
        Ink.draw(text, Ink.keycap, colour: .secondaryLabelColor,
                 x: x + (width - words) / 2, middle: middle, room: words + 1)
    }
}

@MainActor
enum Ink {
    static let body = NSFont.systemFont(ofSize: 14)
    static let bodyBold = NSFont.systemFont(ofSize: 14, weight: .bold)
    static let heading = NSFont.systemFont(ofSize: 13, weight: .semibold)
    static let pill = NSFont.systemFont(ofSize: 12)
    static let chip = NSFont.systemFont(ofSize: 12.5, weight: .bold)
    static let date = NSFont.systemFont(ofSize: 13.5, weight: .bold)
    static let quote = NSFontManager.shared.convert(
        NSFont.systemFont(ofSize: 14), toHaveTrait: .italicFontMask)
    static let action = NSFont.systemFont(ofSize: 13.5, weight: .bold)
    static let time = NSFont.monospacedDigitSystemFont(ofSize: 14, weight: .regular)
    static let timeBold = NSFont.monospacedDigitSystemFont(ofSize: 14, weight: .bold)
    static let badge = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
    static let keycap = NSFont.monospacedSystemFont(ofSize: KeyCapMetrics.fontSize, weight: .regular)
    static let newPill = NSFont.systemFont(ofSize: 11, weight: .bold)
    static let avatar = NSFont.systemFont(ofSize: 8, weight: .semibold)
    static let hint = NSFont.systemFont(ofSize: 12)
    static let bodyItalic = NSFontManager.shared.convert(
        NSFont.systemFont(ofSize: 14), toHaveTrait: .italicFontMask)

    private static let truncating: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.lineBreakMode = .byTruncatingTail
        return style
    }()

    static func width(_ text: String, _ font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    @discardableResult
    static func draw(
        _ text: String, _ font: NSFont, colour: NSColor, x: CGFloat, middle: CGFloat, room: CGFloat
    ) -> CGFloat {
        guard room > 0 else { return 0 }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font, .foregroundColor: colour, .paragraphStyle: truncating,
        ]
        let drawn = min(width(text, font), room)
        let height = ceil(font.ascender - font.descender + font.leading)
        (text as NSString).draw(
            with: NSRect(x: x, y: middle - height / 2, width: drawn, height: height),
            options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine],
            attributes: attributes
        )
        return drawn
    }
}

@MainActor
enum LabelDot {
    static func colour(_ colour: LabelColour?) -> NSColor {
        guard let colour else { return .secondaryLabelColor }
        var components = [CGFloat(colour.red), CGFloat(colour.green), CGFloat(colour.blue), 1]
        return NSColor(colorSpace: .sRGB, components: &components, count: 4)
    }
}

public struct FocusListView: NSViewRepresentable {
    private let table: FocusListTable

    public init(table: FocusListTable) {
        self.table = table
    }

    public func makeNSView(context: Context) -> NSScrollView {
        table.scrollView
    }

    public func updateNSView(_ scroll: NSScrollView, context: Context) {
    }
}
