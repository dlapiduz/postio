import Foundation
import Observation
import PostioFFI

/// The search dropdown's state, as the controller's `FocusDropdown` leaves
/// it (specs/010-focus-search step 2; screens 01 and 03).
///
/// The dropdown is Focus's controller's (`crates/postio-focus/src/
/// dropdown.rs`): which state it is in, what every row and pill says, what
/// running one does. This holds the last view, spelled for drawing -- each
/// key the keymap spells as a Mac keycap -- and decides one thing, the
/// arrows' highlight (009 FR-004): the rows that run, in order; kept on
/// the same row while a new view still draws it, else the view's default;
/// moved wherever a run said (`select`).
///
/// No AppKit (#1264): `DropdownView` draws it, the toolbar's field hands
/// it the keys through `CommandBarModel`.
@MainActor
@Observable
public final class DropdownModel {
    /// One row, as drawn.
    public struct Row: Equatable, Identifiable {
        public let id: UInt64
        public let kind: DropdownRowKindFfi
        public let title: [RunFfi]
        public let detail: [RunFfi]
        public let folder: String?
        public let right: String?
        /// The keycap, spelled for the Mac.
        public let cap: String?
        public let selectable: Bool
    }

    /// One pill, as drawn.
    public struct Pill: Equatable, Identifiable {
        public let id: UInt64
        public let op: String?
        public let label: String
        public let count: String?
        public let cap: String?
    }

    /// One section, as drawn.
    public struct Section: Equatable, Identifiable {
        public let id: Int
        public let title: String
        public let note: String?
        /// The note's keycap, drawn before its words: `⌥⌫`.
        public let noteCap: String?
        public let rows: [Row]
        public let pills: [Pill]
    }

    /// One footer hint: its keycap and its words.
    public struct Hint: Equatable {
        public let cap: String
        public let words: String
    }

    public private(set) var state: DropdownStateFfi = .empty
    public private(set) var sections: [Section] = []
    public private(set) var hints: [Hint] = []
    /// "48 matches · 38 ms".
    public private(set) var count: String?
    /// The highlighted row's token.
    public private(set) var highlighted: UInt64?

    @ObservationIgnored private let engine: CommandBarEngine

    public init(engine: CommandBarEngine) {
        self.engine = engine
    }

    /// The rows the arrows rest on, top to bottom.
    public var selectable: [UInt64] {
        sections.flatMap { $0.rows.filter(\.selectable).map(\.id) }
    }

    /// The highlighted row, when there is one.
    public var highlightedRow: Row? {
        guard let highlighted else { return nil }
        return sections.lazy.flatMap(\.rows).first { $0.id == highlighted }
    }

    /// Draw `view`.
    public func draw(_ view: DropdownViewFfi) {
        state = view.state
        sections = view.sections.enumerated().map { index, section in
            Section(
                id: index, title: section.title, note: section.note,
                noteCap: KeyCapSpelling.cap(section.noteKey),
                rows: section.rows.map { row in
                    Row(
                        id: row.token, kind: row.kind, title: row.title, detail: row.detail,
                        folder: row.folder, right: row.right, cap: KeyCapSpelling.cap(row.key),
                        selectable: row.selectable)
                },
                pills: section.pills.map { pill in
                    Pill(
                        id: pill.token, op: pill.op, label: pill.label, count: pill.count,
                        cap: KeyCapSpelling.cap(pill.key))
                })
        }
        hints = view.footerHints.map { hint in
            Hint(cap: KeyCapSpelling.cap(hint.key) ?? hint.key, words: hint.label)
        }
        count = view.footerCount
        let runs = selectable
        if let moved = view.select, runs.contains(moved) {
            highlighted = moved
        } else if let kept = highlighted, runs.contains(kept) {
            // The same row, redrawn (passages landing): the arrows stay.
        } else {
            highlighted = view.highlight.flatMap { runs.contains($0) ? $0 : nil } ?? runs.first
        }
    }

    /// ↑ or ↓: the next row that runs, stopping at either end.
    public func move(by delta: Int) {
        let runs = selectable
        guard !runs.isEmpty else { return }
        let at = highlighted.flatMap { runs.firstIndex(of: $0) } ?? 0
        let next = runs[min(max(at + delta, 0), runs.count - 1)]
        guard next != highlighted else { return }
        highlighted = next
        engine.focusSearchHighlighted(next)
    }

    /// Return: run the highlighted row.
    public func runHighlighted() {
        guard let highlighted else { return }
        engine.focusBarRun(highlighted)
    }

    /// A click on the row or pill `token`.
    public func run(_ token: UInt64) {
        if selectable.contains(token) { highlighted = token }
        engine.focusBarRun(token)
    }

    /// ⌘↩: every result for what is typed.
    public func showAll() {
        engine.focusSearchShowAll()
    }

    /// ⌥⌫: forget the highlighted recent search. `false` when it is not
    /// one, and the key is the field's own.
    public func forgetHighlighted() -> Bool {
        guard let row = highlightedRow, row.kind == .recent else { return false }
        engine.focusSearchForget(row.id)
        return true
    }

    /// The bar closed, or its lines came back: nothing is kept.
    func forget() {
        state = .empty
        sections = []
        hints = []
        count = nil
        highlighted = nil
    }
}
