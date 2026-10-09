import Foundation
import Observation
import PostioFFI

/// What the results' query controls tell the engine (specs/010-focus-search
/// T068).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `SearchQueryModel` without a store.
public protocol SearchQueryEngine: AnyObject {
    /// A control changed the query: a filter button, a chip's ✕.
    func focusSearchEdit(_ edit: TermEditFfi)
    /// A filter button with a popover was pressed: open it.
    func focusSearchPopover(_ kind: FilterKindFfi)
}

extension PostioSession: SearchQueryEngine {}

/// The results' query, as the field's chips and the filter bar's buttons
/// draw it (design §1 "One query, two controls", §3.1, §3.2).
///
/// The query is the controller's (`crates/postio-focus/src/results.rs`,
/// through `edit::apply`). This holds what the last `FocusQuery` said and
/// hands back what a control did, as a `TermEditFfi`: an operator's
/// keyword and value apart, or a chip's token. It never spells query text,
/// so a chip and its button cannot disagree -- both are redrawn from the
/// one answer.
///
/// No AppKit (#1264): `FilterBarView` and the toolbar's `ChipQueryField`
/// both read it.
@MainActor
@Observable
public final class SearchQueryModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// The chips, the words or the buttons.
        case query
    }

    /// One operator term, as a chip.
    public struct Chip: Equatable, Identifiable {
        /// Its token: what removing it names.
        public let id: UInt32
        /// "from:", tertiary.
        public let op: String
        /// "ada", in label colour.
        public let value: String
        /// Struck through.
        public let excluded: Bool
        /// Ringed.
        public let focused: Bool

        public init(id: UInt32, op: String, value: String, excluded: Bool, focused: Bool) {
            self.id = id
            self.op = op
            self.value = value
            self.excluded = excluded
            self.focused = focused
        }
    }

    /// One filter button.
    public struct Button: Equatable, Identifiable {
        public let kind: FilterKindFfi
        /// "From", or "From: Ada Moreno" once applied.
        public let label: String
        /// Solid.
        public let applied: Bool
        /// Its popover is open: the accent ring.
        public let open: Bool
        public var id: FilterKindFfi { kind }
        /// A plain toggle (no ▾).
        public var isToggle: Bool { kind.toggleTerm != nil }
    }

    public private(set) var chips: [Chip] = []
    /// The plain words after the chips.
    public private(set) var words = ""
    /// "/ to edit".
    public private(set) var hint = ""
    public private(set) var buttons: [Button] = []

    @ObservationIgnored private let engine: SearchQueryEngine

    public init(engine: SearchQueryEngine) {
        self.engine = engine
    }

    /// Apply `event` if it is the query's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusQuery(view):
            chips = view.chips.map {
                Chip(id: $0.token, op: $0.operator, value: $0.value, excluded: $0.excluded, focused: $0.focused)
            }
            words = view.words
            hint = view.hint
            buttons = view.buttons.map {
                Button(kind: $0.kind, label: $0.label, applied: $0.applied, open: $0.open)
            }
            return .query
        case .focusLeaveResults:
            chips = []
            words = ""
            hint = ""
            buttons = []
            return .query
        default:
            return nil
        }
    }

    /// A filter button was clicked. A toggle sends its term; the others
    /// ask for their popover (step 4), which `FocusPopover` then draws.
    public func tap(_ kind: FilterKindFfi) {
        guard let term = kind.toggleTerm else {
            engine.focusSearchPopover(kind)
            return
        }
        engine.focusSearchEdit(.toggle(field: term.field, value: term.value))
    }

    /// A chip's ✕, or Backspace on a selected chip.
    public func remove(_ token: UInt32) {
        engine.focusSearchEdit(.remove(token: token))
    }

    /// Keep the words, drop every operator.
    public func clearFilters() {
        engine.focusSearchEdit(.clearFilters)
    }
}

extension FilterKindFfi {
    /// A toggle button's term, as the boundary takes it apart: the
    /// operator's keyword and its value (`TermEditFfi::Toggle`'s own
    /// examples). `nil` for a button that opens a popover.
    public var toggleTerm: (field: String, value: String)? {
        switch self {
        case .attachment: return ("has", "attachment")
        case .hasAction: return ("has", "action")
        case .unread: return ("is", "unread")
        case .from, .to, .date, .anywhere, .label: return nil
        }
    }
}
