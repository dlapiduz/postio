import Foundation
import Observation
import PostioFFI

public protocol SearchQueryEngine: AnyObject {
    func focusSearchEdit(_ edit: TermEditFfi)
    func focusSearchPopover(_ kind: FilterKindFfi)
}

extension PostioSession: SearchQueryEngine {}

/// The query is the controller's. A control's action is handed back as a `TermEditFfi` and this
/// never spells query text, so a chip and its button cannot disagree. No AppKit (#1264).
@MainActor
@Observable
public final class SearchQueryModel {
    public enum Change: Equatable {
        case query
    }

    public struct Chip: Equatable, Identifiable {
        public let id: UInt32
        public let op: String
        public let value: String
        public let excluded: Bool
        public let focused: Bool

        public init(id: UInt32, op: String, value: String, excluded: Bool, focused: Bool) {
            self.id = id
            self.op = op
            self.value = value
            self.excluded = excluded
            self.focused = focused
        }
    }

    public struct Button: Equatable, Identifiable {
        public let kind: FilterKindFfi
        public let label: String
        public let applied: Bool
        public let open: Bool
        /// `nil` for a toggle (#2090).
        public let cap: String?
        public var id: FilterKindFfi { kind }
        public var isToggle: Bool { kind.toggleTerm != nil }
    }

    public private(set) var chips: [Chip] = []
    public private(set) var words = ""
    public private(set) var hint = ""
    public private(set) var ringed = false
    public private(set) var buttons: [Button] = []

    @ObservationIgnored private let engine: SearchQueryEngine

    public init(engine: SearchQueryEngine) {
        self.engine = engine
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusQuery(view):
            chips = view.chips.map {
                Chip(id: $0.token, op: $0.operator, value: $0.value, excluded: $0.excluded, focused: $0.focused)
            }
            words = view.words
            hint = [KeyCapSpelling.cap(view.hintKey), view.hint].compactMap { $0 }.joined(separator: " ")
            ringed = view.ringed
            buttons = view.buttons.map {
                Button(
                    kind: $0.kind, label: $0.label, applied: $0.applied, open: $0.open,
                    cap: KeyCapSpelling.cap($0.key))
            }
            return .query
        case .focusLeaveResults:
            chips = []
            words = ""
            hint = ""
            ringed = false
            buttons = []
            return .query
        default:
            return nil
        }
    }

    public func tap(_ kind: FilterKindFfi) {
        guard let term = kind.toggleTerm else {
            engine.focusSearchPopover(kind)
            return
        }
        engine.focusSearchEdit(.toggle(field: term.field, value: term.value))
    }

    public func remove(_ token: UInt32) {
        engine.focusSearchEdit(.remove(token: token))
    }

    public func clearFilters() {
        engine.focusSearchEdit(.clearFilters)
    }
}

extension FilterKindFfi {
    /// `nil` for a button that opens a popover.
    public var toggleTerm: (field: String, value: String)? {
        switch self {
        case .attachment: return ("has", "attachment")
        case .hasAction: return ("has", "action")
        case .unread: return ("is", "unread")
        case .from, .to, .date, .anywhere, .label: return nil
        }
    }
}
