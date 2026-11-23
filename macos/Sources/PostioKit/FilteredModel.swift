import Foundation
import Observation
import PostioFFI

public protocol FilteredEngine: AnyObject {
    func focusFilteredPoint(_ index: UInt32)
    func focusFilteredPick(_ index: UInt32, range: Bool)
    func focusFilteredMore()
    func invoke(_ id: String)
    func binding(for command: String) -> String?
}

extension PostioSession: FilteredEngine {}

public enum FilteredCommand {
    public static let restore = "restore_filtered"
    public static let sweep = "sweep_inbox"
    public static let back = "back"
    public static let open = "open_message"

    public static func tab(_ number: Int) -> String { "filtered_tab_\(number)" }
}

/// The controller keeps the tab, rows, keyboard and pages; this holds what the last `FocusFiltered`
/// said and moves nothing:
/// a click is told and the focus moves when `FocusFilteredFocus` says so. The one thing decided
/// here is asking for the next page once per page. No AppKit (#1264).
@MainActor
@Observable
public final class FilteredModel {
    public enum Change: Equatable {
        case open
        case redraw
        case focus
        case selection
        case close
    }

    public struct Tab: Equatable, Identifiable {
        public let id: Int
        public let name: String
        public let count: UInt32
        public let cap: String?
        public let on: Bool
    }

    public struct Row: Equatable, Identifiable {
        public let id: Int
        public let message: Int64
        public let sender: String
        public let subject: String
        public let preview: String?
        public let pill: String
        public let time: String
        public let heading: String?
        public let selected: Bool
        public let messages: UInt32

        public var isThread: Bool { messages > 1 }

        func with(selected: Bool) -> Row {
            Row(
                id: id, message: message, sender: sender, subject: subject, preview: preview,
                pill: pill, time: time, heading: heading, selected: selected, messages: messages)
        }
    }

    public struct Child: Equatable {
        public let initials: String
        public let sender: String
        public let unread: Bool
        public let tag: String?
        public let line: String
        public let placeholder: Bool
        public let attachment: Bool
        public let time: String
        public let draft: Bool
        public let hintKeys: [String]
        public let hintLabel: String?
    }

    public enum Item: Equatable, Identifiable {
        case row(Row, display: Int)
        case child(Child, display: Int, last: Bool)

        public var id: Int {
            switch self {
            case let .row(_, display), let .child(_, display, _): return display
            }
        }
    }

    public struct Open: Equatable {
        public let anchor: Int
        public let children: [Child]
    }

    public struct Hint: Equatable, Identifiable {
        public let cap: String
        public let label: String
        public var id: String { label }
    }

    public private(set) var isOpen = false
    public private(set) var title = ""
    public private(set) var subtitle = ""
    public private(set) var note = ""
    public private(set) var sweepWords = ""
    public private(set) var sweepCap: String?
    public private(set) var restoreWords = ""
    public private(set) var restoreCap: String?
    public private(set) var tabs: [Tab] = []
    public private(set) var rows: [Row] = []
    public private(set) var open: Open?
    public private(set) var items: [Item] = []
    public private(set) var focused: Int?
    public private(set) var footer: [Hint] = []
    public private(set) var bar: ActionBarWords?
    public private(set) var empty: String?

    @ObservationIgnored private var more = false
    @ObservationIgnored private var askedMore = false

    @ObservationIgnored private let engine: FilteredEngine

    public init(engine: FilteredEngine) {
        self.engine = engine
    }

    public var backCap: String? { KeyCapSpelling.cap(engine.binding(for: FilteredCommand.back)) }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case .focusShowFiltered:
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
        case let .focusFilteredSelection(selected, bar):
            guard isOpen else { return nil }
            let boxed = Set(selected)
            rows = rows.map { $0.with(selected: boxed.contains($0.message)) }
            rebuild()
            self.bar = bar.map(Self.words)
            return .selection
        case let .focusFilteredSplice(anchor, children):
            guard isOpen, rows.indices.contains(Int(anchor)) else { return nil }
            open = Open(anchor: Int(anchor), children: children.map(Self.child))
            rebuild()
            return .redraw
        case .focusFilteredUnsplice:
            guard isOpen, open != nil else { return nil }
            open = nil
            rebuild()
            return .redraw
        case .focusCloseSurface(.filtered):
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    public func point(_ index: Int) {
        guard items.indices.contains(index) else { return }
        engine.focusFilteredPoint(UInt32(index))
    }

    public func toggle(_ index: Int) {
        guard items.indices.contains(index) else { return }
        engine.focusFilteredPick(UInt32(index), range: false)
    }

    public func extend(_ index: Int) {
        guard items.indices.contains(index) else { return }
        engine.focusFilteredPick(UInt32(index), range: true)
    }

    public func run(_ command: String) {
        engine.invoke(command)
    }

    public func open(_ index: Int) {
        guard items.indices.contains(index) else { return }
        engine.focusFilteredPoint(UInt32(index))
        engine.invoke(FilteredCommand.open)
    }

    public func chooseTab(_ number: Int) {
        engine.invoke(FilteredCommand.tab(number))
    }

    public func restore() {
        engine.invoke(FilteredCommand.restore)
    }

    public func sweep() {
        engine.invoke(FilteredCommand.sweep)
    }

    public func back() {
        engine.invoke(FilteredCommand.back)
    }

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
                preview: line.preview, pill: line.pill, time: line.time, heading: line.heading,
                selected: line.selected, messages: line.messages)
        }
        open = view.expanded.map { Open(anchor: Int($0.anchor), children: $0.children.map(Self.child)) }
        rebuild()
        bar = view.bar.map(Self.words)
        empty = view.empty
        focused = view.focused.map(Int.init)
        footer = view.footer.map { Hint(cap: KeyCapSpelling.cap($0.key) ?? $0.key, label: $0.label) }
        more = view.more
        if moved || !view.more { askedMore = false }
    }

    private func rebuild() {
        var out: [Item] = []
        out.reserveCapacity(rows.count + (open?.children.count ?? 0))
        for row in rows {
            out.append(.row(row, display: out.count))
            if let open, open.anchor == row.id {
                for (index, child) in open.children.enumerated() {
                    out.append(.child(child, display: out.count, last: index == open.children.count - 1))
                }
            }
        }
        items = out
    }

    private static func child(_ child: ChildRowFfi) -> Child {
        Child(
            initials: child.initials, sender: child.sender, unread: child.bold, tag: child.tag,
            line: child.line, placeholder: child.placeholder, attachment: child.attachment,
            time: child.time, draft: child.draft,
            hintKeys: child.hint?.keys.compactMap { KeyCapSpelling.cap($0) } ?? [],
            hintLabel: child.hint?.label)
    }

    private static func words(_ bar: FilteredBarFfi) -> ActionBarWords {
        ActionBarWords(
            summary: bar.summary,
            actions: bar.actions.map {
                ActionBarWords.Action(
                    command: $0.command, label: $0.label, cap: KeyCapSpelling.cap($0.key),
                    isDefault: $0.primary)
            },
            hints: bar.hints.map { hint in
                let caps = hint.key.split(separator: "/", omittingEmptySubsequences: true)
                    .map(String.init).compactMap { KeyCapSpelling.cap($0) }
                return ActionBarWords.Hint(caps: caps, label: hint.label)
            })
    }

    private func close() {
        bar = nil
        empty = nil
        isOpen = false
        title = ""
        subtitle = ""
        note = ""
        tabs = []
        rows = []
        open = nil
        items = []
        focused = nil
        footer = []
        more = false
        askedMore = false
    }
}
