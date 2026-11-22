import Foundation
import Observation
import PostioFFI

/// Decides one thing, the arrows' highlight (009 FR-004): kept on the same row while a new view
/// still draws it, else the
/// view's default; moved wherever a run said (`select`). No AppKit (#1264).
@MainActor
@Observable
public final class DropdownModel {
    public struct Row: Equatable, Identifiable {
        public let id: UInt64
        public let kind: DropdownRowKindFfi
        public let title: [RunFfi]
        public let detail: [RunFfi]
        public let folder: String?
        public let right: String?
        public let cap: String?
        public let selectable: Bool
        public let initials: String?
        public var whenHighlighted: Hint? = nil
    }

    public struct Pill: Equatable, Identifiable {
        public let id: UInt64
        public let op: String?
        public let label: String
        public let count: String?
        public let fresh: String?
        public let cap: String?
    }

    public struct Section: Equatable, Identifiable {
        public let id: Int
        public let title: String
        public let note: String?
        public let noteCap: String?
        public let rows: [Row]
        public let pills: [Pill]
    }

    public struct Hint: Equatable {
        public let cap: String
        public let words: String
    }

    public private(set) var state: DropdownStateFfi = .empty
    public private(set) var sections: [Section] = []
    public private(set) var hints: [Hint] = []
    public private(set) var count: String?
    public private(set) var ghost: String?
    public private(set) var ghostHint: Hint?
    public private(set) var operatorPrefix: String?
    public private(set) var understood: [UnderstoodTileFfi] = []
    public private(set) var highlighted: UInt64?

    @ObservationIgnored private let engine: CommandBarEngine

    public init(engine: CommandBarEngine) {
        self.engine = engine
    }

    public var selectable: [UInt64] {
        sections.flatMap { $0.rows.filter(\.selectable).map(\.id) }
    }

    static func hint(_ hint: KeyHintFfi) -> Hint {
        Hint(cap: KeyCapSpelling.cap(hint.key) ?? hint.key, words: hint.label)
    }

    public func highlightedHint(_ row: Row) -> Hint? {
        row.id == highlighted ? row.whenHighlighted : nil
    }

    public var highlightedRow: Row? {
        guard let highlighted else { return nil }
        return sections.lazy.flatMap(\.rows).first { $0.id == highlighted }
    }

    public func draw(_ view: DropdownViewFfi) {
        state = view.state
        ghost = view.ghost
        ghostHint = view.ghostHint.map(Self.hint)
        operatorPrefix = view.operatorPrefix
        understood = view.understood
        sections = view.sections.enumerated().map { index, section in
            Section(
                id: index, title: section.title, note: section.note,
                noteCap: KeyCapSpelling.cap(section.noteKey),
                rows: section.rows.map { row in
                    Row(
                        id: row.token, kind: row.kind, title: row.title, detail: row.detail,
                        folder: row.folder, right: row.right, cap: KeyCapSpelling.cap(row.key),
                        selectable: row.selectable, initials: row.initials,
                        whenHighlighted: row.highlighted.map(Self.hint))
                },
                pills: section.pills.map { pill in
                    Pill(
                        id: pill.token, op: pill.op, label: pill.label, count: pill.count,
                        fresh: pill.fresh, cap: KeyCapSpelling.cap(pill.key))
                })
        }
        hints = view.footerHints.map(Self.hint)
        count = view.footerCount
        let runs = selectable
        if let moved = view.select, runs.contains(moved) {
            highlighted = moved
        } else if let kept = highlighted, runs.contains(kept) {
        } else {
            highlighted = view.highlight.flatMap { runs.contains($0) ? $0 : nil } ?? runs.first
        }
    }

    public func move(by delta: Int) {
        let runs = selectable
        guard !runs.isEmpty else { return }
        let at = highlighted.flatMap { runs.firstIndex(of: $0) } ?? 0
        let next = runs[min(max(at + delta, 0), runs.count - 1)]
        guard next != highlighted else { return }
        highlighted = next
        engine.focusSearchHighlighted(next)
    }

    public func runHighlighted() {
        guard let highlighted else { return }
        engine.focusBarRun(highlighted)
    }

    public func run(_ token: UInt64) {
        if selectable.contains(token) { highlighted = token }
        engine.focusBarRun(token)
    }

    public func showAll() {
        engine.focusSearchShowAll()
    }

    public func forgetHighlighted() -> Bool {
        guard let row = highlightedRow, row.kind == .recent else { return false }
        engine.focusSearchForget(row.id)
        return true
    }

    public func excludeHighlighted() -> Bool {
        guard let row = highlightedRow, [.person, .label, .folder].contains(row.kind) else {
            return false
        }
        engine.focusSearchExclude(row.id)
        return true
    }

    func forget() {
        state = .empty
        ghost = nil
        ghostHint = nil
        operatorPrefix = nil
        understood = []
        sections = []
        hints = []
        count = nil
        highlighted = nil
    }
}
