import Foundation
import Observation
import PostioFFI

/// What the capture window tells the engine (specs/009-focus-macos T116).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `CaptureModel` without a store.
public protocol CaptureEngine: AnyObject {
    /// The text field holds `text` now.
    func focusCaptureTyped(_ text: String)
    /// The due day, "YYYY-MM-DD", or `nil` for none: a quick pick.
    func focusCaptureDue(_ day: String?)
    /// The project list's filter holds `text` now.
    func focusCaptureFilter(_ text: String)
    /// The project `token` was chosen.
    func focusCaptureProject(_ token: UInt64)
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
    /// The binding in force for `command`, for its keycap.
    func binding(for command: String) -> String?
}

extension PostioSession: CaptureEngine {}

/// The registry commands capture's controls are.
public enum CaptureCommand {
    public static let task = "capture_task"
    public static let note = "capture_note"
    /// ⌥S: the subject in place of the sentence.
    public static let useSubject = "capture_use_subject"
    /// ⌘P: the project list.
    public static let changeProject = "capture_change_project"
    /// ⌘↩: write the line.
    public static let write = "capture_write"
    /// Cancel and Escape.
    public static let back = "back"
}

/// The capture window, as the controller's intents leave it
/// (specs/009-focus-macos T116, screen 25): the task text verbatim (⌥S
/// for the subject), the due day with quick picks, the suggested project
/// with its reason (⌘P for the list), the exact line as a preview, and
/// ⌘↩ to write it.
///
/// The controller (`crates/postio-focus/src/capture.rs`) keeps the text,
/// the day, the project and the vault's answers, and writes the line; the
/// words are `postio_ui::capture`'s. This holds the last
/// `FocusOpenCapture`/`FocusCapture` whole and hands back what was typed
/// and picked. The text and the filter are set from the view only when
/// the controller's words differ from the field's, so typing is never
/// overwritten by its own echo.
///
/// No AppKit: the window (`PostioAppKit`) and its SwiftUI content read it,
/// and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class CaptureModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// `FocusOpenCapture`: open the window.
        case open
        /// `FocusCapture`: draw it again.
        case redraw
        /// The controller closed it.
        case close
    }

    public private(set) var isOpen = false
    /// The last view, whole.
    public private(set) var view: CaptureViewFfi?
    /// The text field's words.
    public private(set) var text = ""
    /// The project filter's words.
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

    /// Apply `event` if it is capture's, and say what it changed; `nil`
    /// for every other event.
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

    /// The window was closed by the toolkit, not the controller: `true`
    /// once, when it was up, so the engine is told.
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        close()
        return true
    }

    // MARK: the controls

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

    /// The Task/Note segment.
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
