import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// Focus's list on the Mac: a view-based `NSTableView` over
/// `FocusListModel` (specs/009-focus-macos T032, screens 01 and 02).
///
/// An `NSTableView` rather than a SwiftUI `List` for the reasons R10 gives:
/// ten thousand rows at 60 fps, real cell reuse, and nothing measured. Every
/// row's height is its shape's (`FocusRowMetrics`), answered from what the
/// model already knows, so laying the table out never fetches a page.
///
/// **The table's own selection is off.** The cursor and Focus's selection
/// are the controller's and arrive as intents (`FocusIntents`); a click is
/// reported to it (`onPoint`, `onPick`) and moves nothing here. `NSTableView`'s
/// selection would be a second opinion about both, and a blue fill the
/// design does not draw. The cursor is an accent ring drawn by the row
/// view, around that row only, and a selected row shows a checked box in
/// the gutter.
@MainActor
public final class FocusListTable: NSObject {
    public let model: FocusListModel
    public let tableView: NSTableView
    public let scrollView: NSScrollView

    /// Run a row's action -- a click on Reply, Accept, Snooze -- with the
    /// row it was on.
    public var onAction: ((String, Int) -> Void)?

    /// A plain click on a row: `focusPoint`. The ring moves when the
    /// controller's `FocusCursor` comes back, never here.
    public var onPoint: ((Int) -> Void)?

    /// A modified click on a row: `focusPick`, with `range` for ⇧ and a
    /// toggle for ⌘.
    public var onPick: ((Int, Bool) -> Void)?

    /// Whether the list stands at its very top, said when that changes:
    /// `focusAtTop`. An undo that brings rows in above keeps the list there
    /// only if it was there.
    public var onAtTop: ((Bool) -> Void)?

    /// What was last said through `onAtTop`, so a scroll that stays on the
    /// same side of the top says nothing.
    private var saidAtTop: Bool?

    /// How many cells were made rather than reused, so reuse can be
    /// asserted rather than eyeballed (T035).
    public private(set) var cellsCreated = 0

    /// Likewise the row views (the backgrounds, the heading band and the
    /// ring): a 10k-row inbox scrolled end to end makes a screenful of
    /// them, not ten thousand.
    public private(set) var rowViewsCreated = 0

    static let cellIdentifier = NSUserInterfaceItemIdentifier("postio.focus.row")
    static let rowIdentifier = NSUserInterfaceItemIdentifier("postio.focus.rowview")

    /// Rows whose shape was learned while drawing and whose height the
    /// table has not been told about yet.
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

        // A click is the table's `action`, read with the modifiers held:
        // its own selection is off, so this is the only thing a click does.
        tableView.target = self
        tableView.action = #selector(rowClicked)

        scrollView.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(boundsChanged),
            name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
    }

    @objc private func rowClicked() {
        clicked(row: tableView.clickedRow, modifiers: NSApp.currentEvent?.modifierFlags ?? [])
    }

    @objc private func boundsChanged() {
        scrolled()
    }

    // MARK: what the engine says

    /// `FocusListChanged`: the list is a different length, or a different
    /// list. Everything is drawn again.
    public func listChanged(total: UInt32) {
        model.reset(total: total)
        heightsOwed = []
        tableView.reloadData()
        if let cursor = model.cursor { tableView.scrollRowToVisible(cursor) }
        scrolled()
    }

    /// `FocusPageReady`: redraw that page's rows, and tell the table which of
    /// them changed height now that their shapes are known. Never
    /// `reloadData()`: a page landing behind the reader must not move the
    /// list they are looking at.
    public func pageArrived(_ page: UInt32) {
        let changed = model.pageArrived(page)
        guard !changed.isEmpty else { return }
        noteHeights(changed)
        tableView.reloadData(forRowIndexes: changed, columnIndexes: IndexSet(integer: 0))
    }

    /// `KeymapChanged`: every keycap on screen may spell something else.
    public func keymapChanged() {
        model.keymapChanged()
        redrawVisible()
    }

    // MARK: cursor and selection

    /// The row the keyboard is on: the controller's, as its last
    /// `FocusCursor` said (`FocusIntents`).
    public var cursor: Int? { model.cursor }

    /// Where a picker at the row hangs from (specs/009-focus-macos T092,
    /// screen 11): the row's own lines, without its day heading's band,
    /// from the subject column across `width` -- the picker's -- so a
    /// popover centred on it starts at the subject. In the table's
    /// coordinates; `nil` for a row the list does not have.
    public func pickerAnchor(row: Int, width: CGFloat) -> NSRect? {
        guard row >= 0, row < tableView.numberOfRows else { return nil }
        let frame = tableView.rect(ofRow: row)
        let band = model.row(at: row)?.heading == nil ? 0 : CGFloat(FocusRowMetrics.heading)
        let subject = CGFloat(FocusRowMetrics.columns(width: Double(frame.width)).subjectX)
        return NSRect(x: frame.minX + subject, y: frame.minY + band, width: width, height: frame.height - band)
    }

    /// The conversations marked, drawn as the checked box.
    public var selected: Set<Int64> { model.selected }

    /// A click on `row` with `modifiers` held: told to the controller, which
    /// answers with the intents that move the ring and the boxes. ⌘ toggles
    /// the row, ⇧ takes the range from the anchor, a plain click points.
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

    /// Draw what one of the controller's intents changed
    /// (`FocusIntents.apply`): only the rows it moved, and the scroll.
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
            // The day headings come or go, and with them the heights of the
            // rows that drew one.
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

    /// Say whether the list stands at its very top, when that changed.
    func scrolled() {
        // Nothing is remembered as said until somebody is listening.
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

    /// Tell the table about heights learned while drawing, once, after the
    /// pass that learned them: changing heights from inside `viewFor` is
    /// re-entering the layout that asked.
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

    /// Tell the table `rows` changed height, at once. `noteHeightOfRows`
    /// animates by default, so a page landing after a delete re-read the
    /// list slid every row it touched into place: the whole list appeared to
    /// collapse and grow back. Transitions are 100 ms or absent; this one is
    /// absent.
    private func noteHeights(_ rows: IndexSet) {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            context.allowsImplicitAnimation = false
            tableView.noteHeightOfRows(withIndexesChanged: rows)
        }
    }

    /// The cell to draw into: `existing` if AppKit handed one back, else a
    /// new one.
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
        // The engine's count, never an array held here.
        model.count
    }
}

extension FocusListTable: NSTableViewDelegate {
    public func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        // From the model's memory of the row's shape: never the FFI, which
        // would fetch every page of a 10k inbox to lay the table out.
        CGFloat(model.height(at: row))
    }

    public func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        // The cursor and the selection are the controller's (see the type).
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
        cell.show(shown, picked: shown.map { model.isPicked($0.id) } ?? false)
        cell.onAction = { [weak self] command in self?.onAction?(command, row) }
        // A row redrawn in place keeps its row view, which must follow.
        if let rowView = tableView.rowView(atRow: row, makeIfNecessary: false) as? FocusRowView {
            dress(rowView, row: row, shown: shown)
        }
        // A shape learned just now may not be the height the table laid out.
        if CGFloat(model.height(at: row)) != tableView.rect(ofRow: row).height {
            oweHeight(row)
        }
        return cell
    }

    public func tableView(_ tableView: NSTableView, didAdd rowView: NSTableRowView, forRow row: Int) {
        guard let rowView = rowView as? FocusRowView else { return }
        dress(rowView, row: row, shown: (rowView.view(atColumn: 0) as? FocusRowCell)?.shown)
    }

    /// What the row view draws behind the words: the heading's band, the
    /// selected ground, and the cursor's ring.
    private func dress(_ rowView: FocusRowView, row: Int, shown: FocusRowModel?) {
        rowView.isCursor = row == model.cursor
        rowView.heading = shown?.heading != nil
        rowView.isPicked = shown.map { model.isPicked($0.id) } ?? false
    }
}

// MARK: - the row view: backgrounds, the heading band, the ring

/// What sits behind a row's words: the list's surface, the day heading's
/// band, a selected row's neutral ground, the hairline under the row, and
/// the cursor's accent ring around the row (never its heading).
final class FocusRowView: NSTableRowView {
    var heading = false { didSet { if heading != oldValue { needsDisplay = true } } }
    var isCursor = false { didSet { if isCursor != oldValue { needsDisplay = true } } }
    var isPicked = false { didSet { if isPicked != oldValue { needsDisplay = true } } }

    override var isFlipped: Bool { true }

    /// The part of the row below its heading band.
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
            // A neutral ground, never the accent, which is the cursor's.
            // `withAlphaComponent` replaces the label colour's own alpha
            // rather than scaling it, so this is the label ink at 6%:
            // screen 01's faint grey, which the marker's accent words stay
            // readable on in both appearances.
            NSColor.labelColor.withAlphaComponent(0.06).setFill()
            body.fill()
        }
        NSColor.separatorColor.setFill()
        NSRect(x: 0, y: bounds.height - 1, width: bounds.width, height: 1).fill()
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
        // The keyboard's ring, in the accent, around this row only.
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
    }
}

// MARK: - the cell: the words, drawn

/// One row's words, drawn rather than laid out by Auto Layout: the gutter,
/// sender, subject, up to two pills, the preview, the trailing attachment,
/// count and time, and a marked row's second line. Positions come from
/// `FocusRowMetrics`; the only measuring is a line's own width, for the
/// visible rows only, so the pills can follow the subject as on screen 01.
///
/// The actions are real controls (`FocusActionButton`), so they can be
/// clicked and VoiceOver reaches them.
final class FocusRowCell: NSView {
    private(set) var shown: FocusRowModel?
    private var picked = false
    var onAction: ((String) -> Void)?

    private let gutterIcon = NSImageView()
    private let clip = NSImageView()
    private var buttons: [FocusActionButton] = []

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        for image in [gutterIcon, clip] {
            image.imageScaling = .scaleProportionallyDown
            image.isHidden = true
            addSubview(image)
        }
        clip.image = NSImage(systemSymbolName: "paperclip", accessibilityDescription: "Attachment")
        clip.contentTintColor = .secondaryLabelColor
        setAccessibilityElement(true)
        setAccessibilityRole(.row)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    func show(_ row: FocusRowModel?, picked: Bool) {
        shown = row
        self.picked = picked
        let actions = row?.marker?.actions ?? []
        while buttons.count < actions.count {
            let button = FocusActionButton()
            addSubview(button)
            buttons.append(button)
        }
        for (index, button) in buttons.enumerated() {
            if index < actions.count {
                let action = actions[index]
                // The first answer is the one the row is asking for, drawn
                // as a button; the others are quieter (screen 01).
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

    /// The first line's centre: a one-line row's middle, or a marked row's
    /// upper line.
    private var firstLine: CGFloat {
        guard let shown, shown.layout == .twoLine else {
            return top + CGFloat(FocusRowMetrics.oneLine) / 2
        }
        return top + CGFloat(FocusRowMetrics.firstLine)
    }

    private var secondLine: CGFloat { top + CGFloat(FocusRowMetrics.secondLine) }

    override func layout() {
        super.layout()
        let width = bounds.width
        // The gutter: a checked box when selected, a digest's stack, or
        // nothing (a marked row's dot is drawn, not an image).
        if let shown, picked || shown.layout == .digest {
            gutterIcon.image = NSImage(
                systemSymbolName: picked ? "checkmark.square.fill" : "square.stack.3d.up",
                accessibilityDescription: nil
            )
            gutterIcon.contentTintColor = picked ? .labelColor : .secondaryLabelColor
            gutterIcon.frame = NSRect(
                x: CGFloat(FocusRowMetrics.gutterCentre) - 7, y: firstLine - 7, width: 14, height: 14)
            gutterIcon.isHidden = false
        } else {
            gutterIcon.isHidden = true
        }
        if let shown, shown.hasAttachments {
            let at = trailingColumn(of: shown, width: width).clip ?? 0
            clip.frame = NSRect(x: at, y: firstLine - 8, width: 16, height: 16)
            clip.isHidden = false
        } else {
            clip.isHidden = true
        }
        // The actions, from the right edge in.
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
        let middle = firstLine
        let bold = row.bold || row.layout == .digest

        // A marked row's dot, where no box or stack stands.
        if row.marker != nil, !picked {
            NSColor.controlAccentColor.setFill()
            NSBezierPath(ovalIn: NSRect(
                x: CGFloat(FocusRowMetrics.gutterCentre) - 3.5, y: middle - 3.5, width: 7, height: 7
            )).fill()
        }

        // The trailing column first, from the right edge in, so the middle
        // knows its room: the time, the count, the send state (the clip is
        // an image view, placed by `layout`).
        let trailing = trailingColumn(of: row, width: width)
        Ink.draw(row.time, trailing.timeFont, colour: .labelColor, x: trailing.time, middle: middle, room: width)
        if let badge = row.countBadge, let frame = trailing.badge {
            NSColor.separatorColor.setStroke()
            let outline = NSBezierPath(roundedRect: frame.insetBy(dx: 0.5, dy: 0.5), xRadius: 4, yRadius: 4)
            outline.lineWidth = 1
            outline.stroke()
            Ink.draw(badge, Ink.badge, colour: .secondaryLabelColor, x: frame.minX + 5, middle: middle, room: frame.width)
        }
        if let state = row.sendState, let at = trailing.sendState {
            Ink.draw(state, Ink.body, colour: .secondaryLabelColor, x: at, middle: middle, room: width)
        }
        let end = trailing.end

        // The sender, then the subject, each as it arrived.
        let columns = FocusRowMetrics.columns(width: Double(width))
        let x0 = CGFloat(FocusRowMetrics.senderX)
        Ink.draw(
            row.sender, bold ? Ink.bodyBold : Ink.body, colour: .labelColor,
            x: x0, middle: middle, room: min(CGFloat(columns.senderWidth), end - x0))
        var x = CGFloat(columns.subjectX)
        x += Ink.draw(
            row.subject, bold ? Ink.bodyBold : Ink.body, colour: .labelColor,
            x: x, middle: middle, room: end - x)

        // Up to two pills, while they fit.
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

        // The first line, dimmed, in whatever room is left.
        if let preview = row.preview, end - (x + CGFloat(FocusRowMetrics.gap)) >= 40 {
            x += CGFloat(FocusRowMetrics.gap)
            Ink.draw(preview, Ink.body, colour: .secondaryLabelColor, x: x, middle: middle, room: end - x)
        }

        if let marker = row.marker { drawMarker(marker, width: width, x: CGFloat(columns.markerX)) }
    }

    /// Where the first line's trailing things go, from the right edge in:
    /// the time, the count, the send state, the attachment's clip -- and
    /// where the middle of the line has to end.
    private func trailingColumn(of row: FocusRowModel, width: CGFloat) -> (
        timeFont: NSFont, time: CGFloat, badge: NSRect?, sendState: CGFloat?, clip: CGFloat?,
        end: CGFloat
    ) {
        let gap = CGFloat(FocusRowMetrics.gap)
        let timeFont = row.bold || row.layout == .digest ? Ink.timeBold : Ink.time
        var x = width - CGFloat(FocusRowMetrics.trailing) - Ink.width(row.time, timeFont)
        let time = x
        var badge: NSRect?
        if let count = row.countBadge {
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
        return (timeFont, time, badge, sendState, clip, x - gap)
    }

    /// The marker's line: the chip outlined in the accent, the date bold in
    /// the accent, the quote italic in the accent -- or the status, where
    /// the actions would be.
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

// MARK: - an action: words and their key

/// One answer on a marked row: its words in bold and its keycap. The first
/// on a row is outlined on the raised surface; the rest stand quieter, as
/// screen 01 draws Decline beside Accept. Never the accent: that is the
/// marker's (FR-017).
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
        // Taken here, so a click on an answer never reaches the table as a
        // click on the row.
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

// MARK: - the keycap, in AppKit

/// The keycap, drawn: `KeyCapMetrics` for its geometry, the quaternary
/// fill and separator outline, the monospaced face. The same element as
/// SwiftUI's `KeyCap`, for the places that draw rather than lay out.
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

    /// How wide a cap for `text` is.
    public static func width(of text: String) -> CGFloat {
        max(KeyCapMetrics.minWidth, Ink.width(text, Ink.keycap) + 2 * KeyCapMetrics.padding)
    }

    /// Draw a cap for `text` with its left edge at `x`, centred on `middle`,
    /// in the current (flipped) context.
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

// MARK: - type

/// The faces the list sets its words in: the system font for the chrome,
/// its monospaced face for keys, counts and times (FR-018).
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

    /// One line, truncated at its tail.
    private static let truncating: NSParagraphStyle = {
        let style = NSMutableParagraphStyle()
        style.lineBreakMode = .byTruncatingTail
        return style
    }()

    static func width(_ text: String, _ font: NSFont) -> CGFloat {
        ceil((text as NSString).size(withAttributes: [.font: font]).width)
    }

    /// Draw `text` from `x`, centred on `middle`, in no more than `room`.
    /// Answers how wide it drew.
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

/// A label's dot: the colour the user gave the label, or the secondary
/// label colour when it has none. The user's data, not a literal.
@MainActor
enum LabelDot {
    static func colour(_ colour: LabelColour?) -> NSColor {
        guard let colour else { return .secondaryLabelColor }
        var components = [CGFloat(colour.red), CGFloat(colour.green), CGFloat(colour.blue), 1]
        return NSColor(colorSpace: .sRGB, components: &components, count: 4)
    }
}

// MARK: - in SwiftUI

/// `FocusListTable` as SwiftUI holds it.
public struct FocusListView: NSViewRepresentable {
    private let table: FocusListTable

    public init(table: FocusListTable) {
        self.table = table
    }

    public func makeNSView(context: Context) -> NSScrollView {
        table.scrollView
    }

    public func updateNSView(_ scroll: NSScrollView, context: Context) {
        // Nothing to push: the table pulls, and engine events reach it
        // through `FocusListTable` directly.
    }
}
