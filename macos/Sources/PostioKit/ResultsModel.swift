import Foundation
import Observation
import PostioFFI

/// What the results table tells the engine, and asks it
/// (specs/010-focus-search T072).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `ResultsModel` without a store.
public protocol ResultsEngine: AnyObject {
    /// The result at `position`, or `nil` while its page is on its way.
    func focusSearchRow(_ position: UInt64) -> ResultRowFfi?
    /// A click on the result at `position`.
    func focusSearchPoint(_ position: UInt64)
    /// A tab picked by a click.
    func focusSearchTab(_ tab: ResultsTabFfi)
    /// The Sort menu.
    func focusSearchOrder(_ order: ConversationOrderFfi)
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
}

extension PostioSession: ResultsEngine {}

/// The results view, as the controller's `FocusResults` leaves it
/// (specs/010-focus-search T072; design §3.3-3.5, screens 06 and 07).
///
/// The controller (`crates/postio-focus/src/results.rs`) holds the query,
/// the tab, the order, which conversations match, how they are grouped
/// and where the focus ring is. This holds the last frame, spelled for
/// drawing, and works out the one thing the table needs that no event
/// says: its own rows -- a header before each group's rows -- and each
/// one's height, from its group, so laying the table out never reads a
/// row. Rows are read by position when drawn and remembered until a
/// `FocusResultsPage` says theirs changed; a row whose page has not landed
/// is asked for again rather than remembered.
///
/// A click is told to the controller and moves nothing here: the ring
/// moves when `FocusResultsCursor` says.
///
/// No AppKit (#1264): `ResultsTable` and the SwiftUI views read it.
@MainActor
@Observable
public final class ResultsModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// The results are up, in the inbox's place.
        case open
        /// A new frame: the groups, the counts, the timeline.
        case redraw
        /// These table rows' results changed: draw them again.
        case rows(IndexSet)
        /// The focus ring moved off the table row `previous`.
        case cursor(previous: Int?)
        /// The results are gone; the inbox is back.
        case close
    }

    /// What a table row shows.
    public enum Item: Equatable {
        /// The header of `groups[index]`.
        case header(Int)
        /// The result at this position.
        case row(UInt64)
    }

    /// The heights of design §3.4.
    public enum Metrics {
        public static let header: Double = 32
        public static let topHit: Double = 66
        public static let row: Double = 58
    }

    /// One footer or bulk-bar hint.
    public struct Hint: Equatable, Identifiable {
        public let cap: String
        public let label: String
        public var id: String { cap + label }
    }

    public private(set) var isOpen = false
    public private(set) var tabs: [TabFfi] = []
    public private(set) var order: ConversationOrderFfi = .bestMatch
    /// "48 conversations".
    public private(set) var countLine = ""
    /// "12 files · 6 people · last 12 months".
    public private(set) var subLine = ""
    /// The twelve bars, oldest first.
    public private(set) var months: [MonthBarFfi] = []
    /// The timeline's hint: "Matches by month · drag across months to
    /// narrow", or the range selected.
    public private(set) var timelineHint = ""
    /// ⌥←/⌥→ "steps a month", while a range is selected.
    public private(set) var timelineStep: KeyHintFfi?
    public private(set) var groups: [ResultGroupFfi] = []
    /// How many results.
    public private(set) var results: UInt64 = 0
    /// The result with the focus ring.
    public private(set) var cursor: UInt64?
    public private(set) var footerHints: [Hint] = []
    /// "48 conversations · local index · 41 ms".
    public private(set) var footerRight = ""
    /// Results checked: the footer is the bulk bar while above zero.
    public private(set) var selected: UInt64 = 0
    public private(set) var bulk: [Hint] = []

    /// Bumped on every frame, so a view that draws the rows knows to.
    public private(set) var frame = 0

    @ObservationIgnored private var read: [UInt64: ResultRowFfi] = [:]
    /// Each group's header row in the table.
    @ObservationIgnored private var starts: [Int] = []
    @ObservationIgnored private let engine: ResultsEngine

    public init(engine: ResultsEngine) {
        self.engine = engine
    }

    /// Apply `event` if it is the results', and say what it changed; `nil`
    /// for every other event.
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
        case .focusLeaveResults:
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    // MARK: the table's shape

    /// How many table rows: every result and a header for each group.
    public var count: Int {
        guard isOpen else { return 0 }
        return Int(results) + groups.count
    }

    /// What table row `tableRow` shows; `nil` past the end.
    public func item(at tableRow: Int) -> Item? {
        guard tableRow >= 0, tableRow < count else { return nil }
        // The last group whose header is at or above the row.
        guard let index = starts.lastIndex(where: { $0 <= tableRow }) else {
            // Results before any group: none are drawn so, but stay sane.
            return .row(UInt64(tableRow))
        }
        let offset = tableRow - starts[index]
        if offset == 0 { return .header(index) }
        return .row(groups[index].first + UInt64(offset - 1))
    }

    /// The table row the result at `position` is drawn in.
    public func tableRow(of position: UInt64) -> Int? {
        guard position < results else { return nil }
        guard let index = groups.lastIndex(where: { $0.first <= position }) else { return Int(position) }
        return starts[index] + 1 + Int(position - groups[index].first)
    }

    /// The table row the focus ring is in.
    public var cursorRow: Int? { cursor.flatMap(tableRow(of:)) }

    /// Table row `tableRow`'s height: a header's, a top hit's, or a row's.
    public func height(at tableRow: Int) -> Double {
        switch item(at: tableRow) {
        case .header: return Metrics.header
        case let .row(position):
            let group = groups.last { $0.first <= position }
            return group?.topHits == true ? Metrics.topHit : Metrics.row
        case nil: return Metrics.row
        }
    }

    /// The group `tableRow` belongs to.
    public func group(at tableRow: Int) -> ResultGroupFfi? {
        guard let index = starts.lastIndex(where: { $0 <= tableRow }) else { return nil }
        return groups[index]
    }

    /// The result at `position`: read once, then remembered until its page
    /// changes; `nil` while its page is on its way.
    public func row(at position: UInt64) -> ResultRowFfi? {
        if let row = read[position] { return row }
        guard isOpen, position < results, let row = engine.focusSearchRow(position) else { return nil }
        read[position] = row
        return row
    }

    // MARK: the pointer

    /// A click on table row `tableRow`: the controller is told; a header
    /// is not a result.
    public func point(tableRow: Int) {
        guard case let .row(position) = item(at: tableRow) else { return }
        engine.focusSearchPoint(position)
    }

    /// A double click: the ring goes there and the result opens, as ↩.
    public func open(tableRow: Int) {
        guard case let .row(position) = item(at: tableRow) else { return }
        engine.focusSearchPoint(position)
        engine.invoke(ResultsCommand.open)
    }

    /// A tab clicked.
    public func pick(_ tab: ResultsTabFfi) { engine.focusSearchTab(tab) }

    /// The Sort menu.
    public func pick(_ order: ConversationOrderFfi) { engine.focusSearchOrder(order) }

    // MARK: drawing

    private func draw(_ view: ResultsViewFfi) {
        tabs = view.tabs
        order = view.order
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
        var start = 0
        starts = groups.map { group in
            defer { start += 1 + Int(group.rows) }
            return start
        }
        read = [:]
        frame += 1
    }

    private func close() {
        isOpen = false
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
        starts = []
        read = [:]
    }
}

/// The registry commands the results' controls run by name.
public enum ResultsCommand {
    /// ↩, or a double click: open the focused result.
    public static let open = "open_message"
    /// ‹ Inbox, Escape.
    public static let back = "back"
    /// ⌘[ and the swipe right: back through the history.
    public static let historyBack = "history_back"
    /// ⌘] and the swipe left: forward.
    public static let historyForward = "history_forward"
}
