import Foundation
import Observation
import PostioFFI

public protocol RuleEngine: AnyObject {
    func focusRuleQuery(_ text: String)
    func focusRuleMatchInstead()
    func focusRuleLikeThis()
    func focusRuleSchedule(_ schedule: RuleScheduleFfi)
    func focusRuleCreate()
    func invoke(_ id: String)
}

extension PostioSession: RuleEngine {}

/// The controller keeps the draft rule, reads the preview and writes the rule on Create. The query
/// field is set from the view only when the controller's words differ from the field's, so typing
/// is never overwritten by its own echo. No AppKit (#1264).
@MainActor
@Observable
public final class RuleSheetModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
    }

    public private(set) var isOpen = false
    public private(set) var view: RuleViewFfi?
    public private(set) var query = ""

    @ObservationIgnored private let engine: RuleEngine

    public init(engine: RuleEngine) {
        self.engine = engine
    }

    public var createCap: String? { KeyCapSpelling.cap(view?.createKey) }

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

    /// `true` once, when it was up, so the engine is told.
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        close()
        return true
    }

    public func setCadence(_ index: Int) { say { $0.cadence = UInt32(index) } }
    public func setWeekday(_ index: Int) { say { $0.weekday = UInt32(index) } }
    public func setMonthDay(_ day: Int) { say { $0.monthDay = UInt32(day) } }
    public func setTime(_ time: String) { say { $0.at = time } }

    public func typed(_ text: String) {
        guard text != query else { return }
        query = text
        engine.focusRuleQuery(text)
    }

    public func matchInstead() { engine.focusRuleMatchInstead() }
    public func likeThis() { engine.focusRuleLikeThis() }
    public func create() { engine.focusRuleCreate() }
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
