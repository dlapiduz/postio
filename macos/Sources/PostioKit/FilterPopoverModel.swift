import Foundation
import Observation
import PostioFFI

public protocol FilterPopoverEngine: AnyObject {
    func focusSearchPopover(_ kind: FilterKindFfi)
    func focusSearchPopoverToggle(_ token: UInt64, exclude: Bool)
    func focusSearchPopoverFilter(_ text: String)
    func focusSearchPopoverDone(_ apply: Bool)
    func focusSearchDateWords(_ text: String)
    func focusSearchDatePreset(_ token: UInt64)
    func focusSearchMonths(_ first: UInt32, _ last: UInt32)
}

extension PostioSession: FilterPopoverEngine {}

/// A close the controller made (`FocusPopover` with none) is never reported back; one the toolkit
/// made (Esc, a click away) is, once.
/// The toolkit's own: which row the arrows rest on, and what the fields hold while typed in. No
/// AppKit (#1264).
@MainActor
@Observable
public final class FilterPopoverModel {
    public enum Change: Equatable {
        case open(FilterKindFfi)
        case redraw
        case close
    }

    public private(set) var view: PopoverViewFfi?
    public private(set) var highlight: Int?
    public var filterText = ""
    public var wordsText = ""

    @ObservationIgnored public var dismiss: (() -> Void)?

    @ObservationIgnored private let engine: FilterPopoverEngine

    public init(engine: FilterPopoverEngine) {
        self.engine = engine
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        guard case let .focusPopover(next) = event else { return nil }
        guard let next else {
            guard view != nil else { return nil }
            view = nil
            highlight = nil
            return .close
        }
        let opened = view?.kind != next.kind
        view = next
        if opened {
            filterText = next.filter
            wordsText = next.words
            highlight = next.rows.isEmpty ? nil : 0
            return .open(next.kind)
        }
        if let at = highlight {
            highlight = next.rows.isEmpty ? nil : min(at, next.rows.count - 1)
        } else if !next.rows.isEmpty {
            highlight = 0
        }
        return .redraw
    }

    public func closedByToolkit() {
        guard view != nil else { return }
        view = nil
        highlight = nil
        engine.focusSearchPopoverDone(false)
    }

    public func cancel() {
        guard view != nil else { return }
        closedByToolkit()
        dismiss?()
    }

    public func apply() {
        guard view != nil else { return }
        engine.focusSearchPopoverDone(true)
    }

    public func toggle(token: UInt64, exclude: Bool) {
        engine.focusSearchPopoverToggle(token, exclude: exclude)
    }

    public func toggleHighlighted() {
        guard let view, let at = highlight, view.rows.indices.contains(at) else { return }
        engine.focusSearchPopoverToggle(view.rows[at].token, exclude: false)
    }

    public func moveHighlight(by step: Int) {
        guard let view, !view.rows.isEmpty else { return }
        let at = (highlight ?? -1) + step
        highlight = max(0, min(view.rows.count - 1, at))
    }

    public func filter(_ text: String) {
        filterText = text
        engine.focusSearchPopoverFilter(text)
    }

    public func words(_ text: String) {
        wordsText = text
        engine.focusSearchDateWords(text)
    }

    public func preset(_ token: UInt64) {
        wordsText = ""
        engine.focusSearchDatePreset(token)
    }

    public func months(_ first: UInt32, _ last: UInt32) {
        engine.focusSearchMonths(first, last)
    }
}
