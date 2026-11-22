import Foundation
import Observation
import PostioFFI

public protocol CaptureEngine: AnyObject {
    func focusCaptureTyped(_ text: String)
    func focusCaptureDue(_ day: String?)
    func focusCaptureFilter(_ text: String)
    func focusCaptureProject(_ token: UInt64)
    func invoke(_ id: String)
    func binding(for command: String) -> String?
}

extension PostioSession: CaptureEngine {}

public enum CaptureCommand {
    public static let task = "capture_task"
    public static let note = "capture_note"
    public static let useSubject = "capture_use_subject"
    public static let changeProject = "capture_change_project"
    public static let write = "capture_write"
    public static let back = "back"
}

/// The text and the filter are set from the view only when the controller's words differ from the
/// field's, so typing is never
/// overwritten by its own echo. No AppKit, so iOS stays reachable (#1264).
@MainActor
@Observable
public final class CaptureModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
    }

    public private(set) var isOpen = false
    public private(set) var view: CaptureViewFfi?
    public private(set) var text = ""
    public private(set) var filter = ""

    @ObservationIgnored private let engine: CaptureEngine

    public init(engine: CaptureEngine) {
        self.engine = engine
    }

    public var subjectCap: String? { KeyCapSpelling.cap(view?.subjectKey) }
    public var projectCap: String? { KeyCapSpelling.cap(view?.projectKey) }
    public var buttonCap: String? { KeyCapSpelling.cap(view?.buttonKey) }
    public var taskCap: String? { KeyCapSpelling.cap(engine.binding(for: CaptureCommand.task)) }
    public var noteCap: String? { KeyCapSpelling.cap(engine.binding(for: CaptureCommand.note)) }
    public var backCap: String? { KeyCapSpelling.cap(engine.binding(for: CaptureCommand.back)) }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOpenCapture(view):
            let fresh = !isOpen
            isOpen = true
            draw(view)
            return fresh ? .open : .redraw
        case let .focusCapture(view):
            guard isOpen else { return nil }
            draw(view)
            return .redraw
        case .focusCloseSurface(.capture):
            guard isOpen else { return nil }
            close()
            return .close
        default:
            return nil
        }
    }

    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        close()
        return true
    }

    public func typed(_ words: String) {
        guard words != text else { return }
        text = words
        engine.focusCaptureTyped(words)
    }

    public func filtered(_ words: String) {
        guard words != filter else { return }
        filter = words
        engine.focusCaptureFilter(words)
    }

    public func pick(_ pick: CapturePickFfi) { engine.focusCaptureDue(pick.day) }
    public func choose(_ token: UInt64) { engine.focusCaptureProject(token) }

    public func choose(mode: CaptureModeFfi) {
        engine.invoke(mode == .task ? CaptureCommand.task : CaptureCommand.note)
    }

    public func useSubject() { engine.invoke(CaptureCommand.useSubject) }
    public func changeProject() { engine.invoke(CaptureCommand.changeProject) }
    public func write() { engine.invoke(CaptureCommand.write) }
    public func cancel() { engine.invoke(CaptureCommand.back) }

    private func draw(_ view: CaptureViewFfi) {
        self.view = view
        if view.text != text { text = view.text }
        if view.filter != filter { filter = view.filter }
    }

    private func close() {
        isOpen = false
        view = nil
        text = ""
        filter = ""
    }
}
