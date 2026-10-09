import Foundation
import Observation
import PostioFFI

/// What the digest-this-sender sheet tells the engine
/// (specs/009-focus-macos T115).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `RuleSheetModel` without a store.
public protocol RuleEngine: AnyObject {
    /// The query field holds `text` now.
    func focusRuleQuery(_ text: String)
    /// "Match a list or a search instead…".
    func focusRuleMatchInstead()
    /// "Digest mail like this".
    func focusRuleLikeThis()
    /// The schedule, as the controls hold it now.
    func focusRuleSchedule(_ schedule: RuleScheduleFfi)
    /// Create (or Save).
    func focusRuleCreate()
    /// A registry command: Cancel is Back.
    func invoke(_ id: String)
}

extension PostioSession: RuleEngine {}

/// The digest-this-sender sheet, as the controller's intents leave it
/// (specs/009-focus-macos T115, screen 24): the sender pre-filled, daily,
/// weekly or monthly with the day and the time, a preview of what the
/// rule would have caught, and "Match a list or a search instead…".
///
/// The controller (`crates/postio-focus/src/digest.rs`) keeps the draft
/// rule, reads the preview, and writes the rule on Create; the words are
/// `postio_ui::digest`'s. This holds the last `FocusOpenRule`/`FocusRule`
/// whole, and says every change of a control as the whole schedule. The
/// query field is set from the view only when the controller's words
/// differ from the field's, so typing is never overwritten by its own
/// echo.
///
/// No AppKit: the sheet (`PostioAppKit`) and its SwiftUI content read it,
/// and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class RuleSheetModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// `FocusOpenRule`: show the sheet.
        case open
        /// `FocusRule`: draw it again.
        case redraw
        /// The controller closed it.
        case close
    }

    public private(set) var isOpen = false
    /// The last view, whole.
    public private(set) var view: RuleViewFfi?
    /// The query field's words, as the controller last drew them or the
    /// person last typed them.
    public private(set) var query = ""

    @ObservationIgnored private let engine: RuleEngine

    public init(engine: RuleEngine) {
        self.engine = engine
    }

    /// Create's keycap.
    public var createCap: String? { KeyCapSpelling.cap(view?.createKey) }

    /// Apply `event` if it is the sheet's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOpenRule(view):
            let fresh = !isOpen
            isOpen = true
            draw(view)
            return fresh ? .open : .redraw
        case let .focusRule(view):
            guard isOpen else { return nil }
            draw(view)
            return .redraw
        case .focusCloseSurface(.dialog):
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    /// The sheet was closed by the toolkit, not the controller: `true`
    /// once, when it was up, so the engine is told.
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        close()
        return true
    }

    // MARK: the controls

    public func setCadence(_ index: Int) { say { $0.cadence = UInt32(index) } }
    public func setWeekday(_ index: Int) { say { $0.weekday = UInt32(index) } }
    /// The day of the month from 0.
    public func setMonthDay(_ day: Int) { say { $0.monthDay = UInt32(day) } }
    /// "HH:MM", 24-hour.
    public func setTime(_ time: String) { say { $0.at = time } }

    /// The query field holds `text` now.
    public func typed(_ text: String) {
        guard text != query else { return }
        query = text
        engine.focusRuleQuery(text)
    }

    public func matchInstead() { engine.focusRuleMatchInstead() }
    public func likeThis() { engine.focusRuleLikeThis() }
    public func create() { engine.focusRuleCreate() }
    /// Cancel, and Escape: the controller's Back closes the sheet.
    public func cancel() { engine.invoke("back") }

    private func say(_ change: (inout RuleScheduleFfi) -> Void) {
        guard var schedule = view?.schedule else { return }
        change(&schedule)
        engine.focusRuleSchedule(schedule)
    }

    private func draw(_ view: RuleViewFfi) {
        self.view = view
        if let words = view.query, words != query { query = words }
        if view.query == nil { query = "" }
    }

    private func close() {
        isOpen = false
        view = nil
        query = ""
    }
}
