import Foundation
import Observation
import PostioFFI

public protocol ResultsEngine: AnyObject {
    /// The result at `position`, or `nil` while its page is on its way.
    func focusSearchRow(_ position: UInt64) -> ResultRowFfi?
    /// The Files tab's card at `position`, or `nil` past the last.
    func focusSearchFile(_ position: UInt64) -> FileCardFfi?
    /// The People tab's row at `position`, or `nil` past the last.
    func focusSearchPerson(_ position: UInt64) -> PersonRowFfi?
    func focusSearchPoint(_ position: UInt64)
    func focusSearchTab(_ tab: ResultsTabFfi)
    func focusSearchOrder(_ order: ConversationOrderFfi)
    func invoke(_ id: String)
}

extension PostioSession: ResultsEngine {}

/// Holds the last frame and works out the table's own rows (a header before each group) and
/// heights, so layout never reads a row. Rows are read by position and remembered until a
/// `FocusResultsPage` says theirs changed; a row whose page has not landed is asked for again. A
/// click moves nothing here: the ring moves when `FocusResultsCursor` says. No AppKit (#1264).
@MainActor
@Observable
public final class ResultsModel {
    public enum Change: Equatable {
        case open
        case redraw
        case rows(IndexSet)
        case cursor(previous: Int?)
        case spliced(removed: Range<Int>?, inserted: Range<Int>?)
        case close
    }

    public enum Item: Equatable {
        case header(Int)
        case row(UInt64)
        case child(Int)
    }

    public enum Metrics {
        public static let header: Double = 32
        public static let topHit: Double = 66
        public static let row: Double = 58
        public static let child: Double = 38
        public static let gap: Double = 30
    }

    public struct Hint: Equatable, Identifiable {
        public let cap: String
        public let label: String
        public var id: String { cap + label }
    }

    public private(set) var isOpen = false
    public private(set) var tabs: [TabFfi] = []
    public private(set) var order: ConversationOrderFfi = .bestMatch
    public private(set) var sortable = true
    public private(set) var countLine = ""
    public private(set) var subLine = ""
    public private(set) var months: [MonthBarFfi] = []
    public private(set) var timelineHint = ""
    public private(set) var timelineStep: KeyHintFfi?
    public private(set) var groups: [ResultGroupFfi] = []
    public private(set) var results: UInt64 = 0
    public private(set) var cursor: UInt64?
    public private(set) var splice: ResultSpliceFfi?
    public private(set) var footerHints: [Hint] = []
    public private(set) var footerRight = ""
    public private(set) var selected: UInt64 = 0
    public private(set) var bulk: [Hint] = []
    public private(set) var selectAll: Hint?

    /// Bumped on every frame, so a view that draws the rows knows to.
    public private(set) var frame = 0

    public private(set) var filesHeader: FilesHeaderFfi?

    public var isFiles: Bool { filesHeader != nil }

    public var cardCount: Int { isFiles ? Int(results) : 0 }

    public var isPeople: Bool { isOpen && tabs.contains { $0.selected && $0.tab == .people } }

    public var personCount: Int { isPeople ? Int(results) : 0 }

    @ObservationIgnored private var read: [UInt64: ResultRowFfi] = [:]
    @ObservationIgnored private var cards: [UInt64: FileCardFfi] = [:]
    @ObservationIgnored private var persons: [UInt64: PersonRowFfi] = [:]
    @ObservationIgnored private var starts: [Int] = []
    @ObservationIgnored private let engine: ResultsEngine

    public init(engine: ResultsEngine) {
        self.engine = engine
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusResults(view):
            let was = isOpen
            draw(view)
            isOpen = true
            return was ? .redraw : .open
        case let .focusResultsPage(first, count):
            guard isOpen, count > 0 else { return nil }
            var rows = IndexSet()
            let last = first.addingReportingOverflow(count).overflow ? UInt64.max : first + count
            for position in first..<min(last, results) {
                read[position] = nil
                if let row = tableRow(of: position) { rows.insert(row) }
            }
            return .rows(rows)
        case let .focusResultsCursor(position):
            guard isOpen else { return nil }
            let previous = cursorRow
            cursor = position
            return .cursor(previous: previous)
        case let .focusResultSplice(next):
            guard isOpen else { return nil }
            return spliced(to: next)
        case let .focusResultChildFocus(child):
            guard isOpen, splice != nil else { return nil }
            let previous = cursorRow
            splice?.focused = child
            return .cursor(previous: previous)
        case .focusLeaveResults:
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    private var childCount: Int { splice?.children.count ?? 0 }

    private var anchorBase: Int? { splice.flatMap { baseRow(of: $0.anchor) } }

    public var count: Int {
        guard isOpen, !isFiles, !isPeople else { return 0 }
        return Int(results) + groups.count + childCount
    }

    public func item(at tableRow: Int) -> Item? {
        guard tableRow >= 0, tableRow < count else { return nil }
        if let anchor = anchorBase, childCount > 0 {
            if tableRow > anchor, tableRow <= anchor + childCount { return .child(tableRow - anchor - 1) }
            if tableRow > anchor + childCount { return baseItem(at: tableRow - childCount) }
        }
        return baseItem(at: tableRow)
    }

    private func baseItem(at tableRow: Int) -> Item? {
        guard let index = starts.lastIndex(where: { $0 <= tableRow }) else {
            return .row(UInt64(tableRow))
        }
        let offset = tableRow - starts[index]
        if offset == 0 { return .header(index) }
        return .row(groups[index].first + UInt64(offset - 1))
    }

    private func baseRow(of position: UInt64) -> Int? {
        guard position < results else { return nil }
        guard let index = groups.lastIndex(where: { $0.first <= position }) else { return Int(position) }
        return starts[index] + 1 + Int(position - groups[index].first)
    }

    public func tableRow(of position: UInt64) -> Int? {
        guard let base = baseRow(of: position) else { return nil }
        if let anchor = anchorBase, base > anchor { return base + childCount }
        return base
    }

    public var cursorRow: Int? {
        if let focused = splice?.focused, let anchor = anchorBase, Int(focused) < childCount {
            return anchor + 1 + Int(focused)
        }
        return cursor.flatMap(tableRow(of:))
    }

    public func height(at tableRow: Int) -> Double {
        switch item(at: tableRow) {
        case .header: return Metrics.header
        case let .row(position):
            let group = groups.last { $0.first <= position }
            return group?.topHits == true ? Metrics.topHit : Metrics.row
        case let .child(index):
            if case .gap = splice?.children[index] { return Metrics.gap }
            return Metrics.child
        case nil: return Metrics.row
        }
    }

    public func group(at tableRow: Int) -> ResultGroupFfi? {
        var base = tableRow
        if let anchor = anchorBase, childCount > 0 {
            if tableRow > anchor + childCount { base -= childCount } else if tableRow > anchor { base = anchor }
        }
        guard let index = starts.lastIndex(where: { $0 <= base }) else { return nil }
        return groups[index]
    }

    public func child(at index: Int) -> ResultChildFfi? {
        guard let children = splice?.children, children.indices.contains(index) else { return nil }
        return children[index]
    }

    /// The result at `position`: read once, then remembered until its page
    /// changes; `nil` while its page is on its way.
    public func row(at position: UInt64) -> ResultRowFfi? {
        if let row = read[position] { return row }
        guard isOpen, position < results, let row = engine.focusSearchRow(position) else { return nil }
        read[position] = row
        return row
    }

    public func card(at position: UInt64) -> FileCardFfi? {
        if let card = cards[position] { return card }
        guard isOpen, isFiles, position < results, let card = engine.focusSearchFile(position) else { return nil }
        cards[position] = card
        return card
    }

    public func point(card position: UInt64) {
        guard isFiles, position < results else { return }
        engine.focusSearchPoint(position)
    }

    public func open(card position: UInt64) {
        point(card: position)
        engine.invoke(ResultsCommand.open)
    }

    /// `cursor` moves without a frame, so where the ring is comes from there.
    public func person(at position: UInt64) -> PersonRowFfi? {
        if let person = persons[position] { return person }
        guard isPeople, position < results, let person = engine.focusSearchPerson(position) else { return nil }
        persons[position] = person
        return person
    }

    public func point(person position: UInt64) {
        guard isPeople, position < results else { return }
        engine.focusSearchPoint(position)
    }

    public func open(person position: UInt64) {
        point(person: position)
        engine.invoke(ResultsCommand.open)
    }

    /// A header is not a result and a line under one has no click of its own (the keys reach it).
    public func point(tableRow: Int) {
        guard case let .row(position) = item(at: tableRow) else { return }
        engine.focusSearchPoint(position)
    }

    public func open(tableRow: Int) {
        guard case let .row(position) = item(at: tableRow) else { return }
        engine.focusSearchPoint(position)
        engine.invoke(ResultsCommand.open)
    }

    public func pick(_ tab: ResultsTabFfi) { engine.focusSearchTab(tab) }

    public func pick(_ order: ConversationOrderFfi) { engine.focusSearchOrder(order) }

    private func draw(_ view: ResultsViewFfi) {
        tabs = view.tabs
        order = view.order
        sortable = view.sortable
        countLine = view.countLine
        subLine = view.subLine
        months = view.months
        timelineHint = view.timelineHint
        timelineStep = view.timelineStep
        groups = view.groups
        results = view.rows
        cursor = view.cursor
        footerHints = view.footerHints.map { Hint(cap: KeyCapSpelling.cap($0.key) ?? $0.key, label: $0.label) }
        footerRight = view.footerRight
        selected = view.selected
        bulk = view.bulk.map { Hint(cap: KeyCapSpelling.cap($0.key) ?? $0.key, label: $0.label) }
        selectAll = view.selectAll.map { Hint(cap: KeyCapSpelling.cap($0.key) ?? $0.key, label: $0.label) }
        filesHeader = view.files
        cards = [:]
        persons = [:]
        var start = 0
        starts = groups.map { group in
            defer { start += 1 + Int(group.rows) }
            return start
        }
        read = [:]
        frame += 1
    }

    private func spliced(to next: ResultSpliceFfi?) -> Change {
        var removed: Range<Int>?
        if let anchor = anchorBase, childCount > 0 { removed = (anchor + 1)..<(anchor + 1 + childCount) }
        splice = next
        var inserted: Range<Int>?
        if let next, let anchor = baseRow(of: next.anchor), !next.children.isEmpty {
            inserted = (anchor + 1)..<(anchor + 1 + next.children.count)
        }
        return .spliced(removed: removed, inserted: inserted)
    }

    private func close() {
        isOpen = false
        splice = nil
        tabs = []
        countLine = ""
        subLine = ""
        months = []
        timelineHint = ""
        timelineStep = nil
        groups = []
        results = 0
        cursor = nil
        footerHints = []
        footerRight = ""
        selected = 0
        bulk = []
        selectAll = nil
        starts = []
        read = [:]
        filesHeader = nil
        cards = [:]
        persons = [:]
    }
}

public enum ResultsCommand {
    public static let open = "open_message"
    public static let back = "back"
    public static let historyBack = "history_back"
    public static let historyForward = "history_forward"
}
