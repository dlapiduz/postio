import Foundation
import Observation
import PostioFFI

/// What a filter popover tells the engine (specs/010-focus-search T083).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `FilterPopoverModel` without a store.
public protocol FilterPopoverEngine: AnyObject {
    /// A filter button with a popover was pressed.
    func focusSearchPopover(_ kind: FilterKindFfi)
    /// Space or a click on row `token`; `exclude` with ⌥ held.
    func focusSearchPopoverToggle(_ token: UInt64, exclude: Bool)
    /// The popover's own search field.
    func focusSearchPopoverFilter(_ text: String)
    /// ↩ (`apply`), or Esc and a click away.
    func focusSearchPopoverDone(_ apply: Bool)
    /// The Date popover's plain words.
    func focusSearchDateWords(_ text: String)
    /// The Date popover's preset `token`.
    func focusSearchDatePreset(_ token: UInt64)
    /// A drag across a month chart ended over bars `first...last`.
    func focusSearchMonths(_ first: UInt32, _ last: UInt32)
}

extension PostioSession: FilterPopoverEngine {}

/// The filter popover that is up, as the controller's last `FocusPopover`
/// drew it (design §3.6, screens 08 and 09).
///
/// The controller (`crates/postio-focus/src/results.rs`) holds which popover
/// is open, the query it will put back, what its rows are and which are
/// checked; every check, preset and word goes down and the next
/// `FocusPopover` redraws it whole. What is the Mac's is the toolkit's
/// own: which row the arrows rest on (the highlight Space toggles), and
/// what the fields hold while they are typed in.
///
/// A close the controller made (`FocusPopover` with none) is never
/// reported back; one the toolkit made -- Esc, a click away -- is, once.
///
/// No AppKit (#1264): `FilterPopover` presents it and the SwiftUI views in
/// `FilterPopoverViews.swift` draw it.
@MainActor
@Observable
public final class FilterPopoverModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// A popover opened: hang it from this button.
        case open(FilterKindFfi)
        /// The one up was drawn again.
        case redraw
        /// It closed.
        case close
    }

    /// The popover, whole; `nil` while none is up.
    public private(set) var view: PopoverViewFfi?
    /// The row the arrows rest on, an index into `view.rows`.
    public private(set) var highlight: Int?
    /// What the list popover's field holds.
    public var filterText = ""
    /// What the Date popover's field holds.
    public var wordsText = ""

    /// Called when the popover's own content asked to close (Esc in its
    /// field): whoever presents it takes it down.
    @ObservationIgnored public var dismiss: (() -> Void)?

    @ObservationIgnored private let engine: FilterPopoverEngine

    public init(engine: FilterPopoverEngine) {
        self.engine = engine
    }

    /// Apply `event` if it is the popover's, and say what it changed; `nil`
    /// for every other event.
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

    // MARK: what happened in it

    /// The toolkit closed it -- Esc, a click away. Reported once, and not
    /// at all when the controller closed it first.
    public func closedByToolkit() {
        guard view != nil else { return }
        view = nil
        highlight = nil
        engine.focusSearchPopoverDone(false)
    }

    /// Esc in the popover's own field: as a click away, and its presenter
    /// takes it down.
    public func cancel() {
        guard view != nil else { return }
        closedByToolkit()
        dismiss?()
    }

    /// ↩: keep what it previewed. The controller closes it.
    public func apply() {
        guard view != nil else { return }
        engine.focusSearchPopoverDone(true)
    }

    /// A click on row `token`; `exclude` with ⌥ held.
    public func toggle(token: UInt64, exclude: Bool) {
        engine.focusSearchPopoverToggle(token, exclude: exclude)
    }

    /// Space: the highlighted row.
    public func toggleHighlighted() {
        guard let view, let at = highlight, view.rows.indices.contains(at) else { return }
        engine.focusSearchPopoverToggle(view.rows[at].token, exclude: false)
    }

    /// The arrows.
    public func moveHighlight(by step: Int) {
        guard let view, !view.rows.isEmpty else { return }
        let at = (highlight ?? -1) + step
        highlight = max(0, min(view.rows.count - 1, at))
    }

    /// The list popover's field changed.
    public func filter(_ text: String) {
        filterText = text
        engine.focusSearchPopoverFilter(text)
    }

    /// The Date popover's field changed.
    public func words(_ text: String) {
        wordsText = text
        engine.focusSearchDateWords(text)
    }

    /// A Date preset clicked.
    public func preset(_ token: UInt64) {
        wordsText = ""
        engine.focusSearchDatePreset(token)
    }

    /// A drag across the Date popover's chart ended.
    public func months(_ first: UInt32, _ last: UInt32) {
        engine.focusSearchMonths(first, last)
    }
}
