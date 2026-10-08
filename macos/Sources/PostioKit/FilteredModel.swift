import Foundation
import Observation
import PostioFFI

/// What Filtered tells the engine (specs/009-focus-macos T113).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `FilteredModel` without a store.
public protocol FilteredEngine: AnyObject {
    /// Row `index` was clicked: the controller puts the keyboard there.
    func focusFilteredPoint(_ index: UInt32)
    /// The end of the rows read so far was reached: read the next page.
    func focusFilteredMore()
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
    /// The binding in force for `command`, for its keycap.
    func binding(for command: String) -> String?
}

extension PostioSession: FilteredEngine {}

/// The registry commands Filtered's buttons are.
public enum FilteredCommand {
    /// Restore the focused row, and never filter its sender again (`R`).
    public static let restore = "restore_filtered"
    /// "Sweep the inbox…".
    public static let sweep = "sweep_inbox"
    /// Leave Filtered: the header's "‹ Inbox", Escape.
    public static let back = "back"
    /// Open the focused row's message over Filtered (Return).
    public static let open = "open_message"

    /// The `n`th tab's command, `n` from 1 to 7.
    public static func tab(_ number: Int) -> String { "filtered_tab_\(number)" }
}

/// Filtered, as the controller's intents leave it (specs/009-focus-macos
/// T113, screen 21): the mail filtering archived as it arrived, a reason on
/// each row, tabs `1`-`7` by reason with their counts, `R` to restore.
///
/// The controller (`crates/postio-focus/src/filtered.rs`) keeps which tab
/// shows, which row has the keyboard, the pages read and what each key
/// does; the words are `postio_ui::filtered`'s. This holds what the last
/// `FocusFiltered` said, spelled for drawing, and hands back what the
/// pointer did. It moves nothing itself: a click is told, and the focus
/// moves when `FocusFilteredFocus` says so.
///
/// The one thing decided here is when to ask for the next page: once the
/// end of the rows on screen is reached, once per page.
///
/// No AppKit: the main window draws `FilteredView` from it, and iOS must
/// stay reachable (#1264).
@MainActor
@Observable
public final class FilteredModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// Filtered is up, in the list's place.
        case open
        /// Its tabs, rows or words changed.
        case redraw
        /// The keyboard moved to another row.
        case focus
        /// Filtered is gone; the list is back.
        case close
    }

    /// One reason tab.
    public struct Tab: Equatable, Identifiable {
        /// Its number, 1 to 7: what its key and its command name.
        public let id: Int
        public let name: String
        public let count: UInt32
        /// Its number key, as a keycap.
        public let cap: String?
        /// Whether it is the one showing.
        public let on: Bool
    }

    /// One filtered message.
    public struct Row: Equatable, Identifiable {
        /// Its place among the rows: what a click tells the controller.
        public let id: Int
        public let message: Int64
        public let sender: String
        public let subject: String
        public let preview: String?
        /// The reason: "notification · Forge", "spam".
        public let pill: String
        public let time: String
        /// The day heading above it, on the first row of each day.
        public let heading: String?
    }

    /// One footer hint.
    public struct Hint: Equatable, Identifiable {
        public let cap: String
        public let label: String
        public var id: String { label }
    }

    public private(set) var isOpen = false
    /// "Filtered".
    public private(set) var title = ""
    /// "Archived automatically · newest first".
    public private(set) var subtitle = ""
    /// Right of the tabs: "Nothing here is deleted automatically · …" (C4).
    public private(set) var note = ""
    public private(set) var sweepWords = ""
    public private(set) var sweepCap: String?
    /// The focused row's button: "Restore, never filter this sender".
    public private(set) var restoreWords = ""
    public private(set) var restoreCap: String?
    public private(set) var tabs: [Tab] = []
    public private(set) var rows: [Row] = []
    /// The row with the keyboard.
    public private(set) var focused: Int?
    public private(set) var footer: [Hint] = []

    /// Whether the rows on screen may not be all there is.
    @ObservationIgnored private var more = false
    /// Whether the next page has been asked for since these rows landed.
    @ObservationIgnored private var askedMore = false

    @ObservationIgnored private let engine: FilteredEngine

    public init(engine: FilteredEngine) {
        self.engine = engine
    }

    /// The keycap for "‹ Inbox": Back's binding.
    public var backCap: String? { KeyCapSpelling.cap(engine.binding(for: FilteredCommand.back)) }

    /// Apply `event` if it is Filtered's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case .focusShowFiltered:
            // `g f`: the view follows. Up now, so the list gives way at once.
            let was = isOpen
            isOpen = true
            return was ? nil : .open
        case let .focusFiltered(view):
            let was = isOpen
            draw(view)
            isOpen = true
            return was ? .redraw : .open
        case let .focusFilteredFocus(index):
            guard isOpen else { return nil }
            focused = index.map(Int.init)
            return .focus
        case .focusCloseSurface(.filtered):
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    /// Row `index` was clicked.
    public func point(_ index: Int) {
        guard rows.indices.contains(index) else { return }
        engine.focusFilteredPoint(UInt32(index))
    }

    /// Row `index` was double-clicked: the keyboard goes there and its
    /// message opens over Filtered, as Return would.
    public func open(_ index: Int) {
        guard rows.indices.contains(index) else { return }
        engine.focusFilteredPoint(UInt32(index))
        engine.invoke(FilteredCommand.open)
    }

    /// Tab `number` (1 to 7) was clicked.
    public func chooseTab(_ number: Int) {
        engine.invoke(FilteredCommand.tab(number))
    }

    /// The focused row's Restore was clicked.
    public func restore() {
        engine.invoke(FilteredCommand.restore)
    }

    /// "Sweep the inbox…" was clicked.
    public func sweep() {
        engine.invoke(FilteredCommand.sweep)
    }

    /// "‹ Inbox" was clicked.
    public func back() {
        engine.invoke(FilteredCommand.back)
    }

    /// The last row on screen was reached: ask for the next page, once.
    public func reachedEnd() {
        guard isOpen, more, !askedMore else { return }
        askedMore = true
        engine.focusFilteredMore()
    }

    private func draw(_ view: FilteredViewFfi) {
        let moved = view.rows.map(\.message) != rows.map(\.message)
        title = view.title
        subtitle = view.subtitle
        note = view.note
        sweepWords = view.sweep
        sweepCap = KeyCapSpelling.cap(view.sweepKey)
        restoreWords = view.restore
        restoreCap = KeyCapSpelling.cap(view.restoreKey)
        tabs = view.tabs.enumerated().map { index, tab in
            Tab(id: index + 1, name: tab.name, count: tab.count, cap: KeyCapSpelling.cap(tab.key), on: tab.on)
        }
        rows = view.rows.enumerated().map { index, line in
            Row(
                id: index, message: line.message, sender: line.sender, subject: line.subject,
                preview: line.preview, pill: line.pill, time: line.time, heading: line.heading)
        }
        focused = view.focused.map(Int.init)
        footer = view.footer.map { Hint(cap: KeyCapSpelling.cap($0.key) ?? $0.key, label: $0.label) }
        more = view.more
        // A page landed (or a tab changed what is listed): the end is new.
        if moved || !view.more { askedMore = false }
    }

    private func close() {
        isOpen = false
        title = ""
        subtitle = ""
        note = ""
        tabs = []
        rows = []
        focused = nil
        footer = []
        more = false
        askedMore = false
    }
}
