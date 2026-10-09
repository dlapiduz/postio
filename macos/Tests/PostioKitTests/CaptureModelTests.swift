import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The capture window, as the controller's intents leave it
/// (specs/009-focus-macos T116, screen 25).
///
/// The controller keeps the text, the due day, the project and the
/// vault's answers, and writes the exact line. The Mac draws every
/// `FocusOpenCapture`/`FocusCapture` whole and hands back what was typed,
/// a day or a project picked, and the buttons as their commands.
@MainActor
struct CaptureModelTests {
    final class Engine: CaptureEngine {
        var typed: [String] = []
        var days: [String?] = []
        var filters: [String] = []
        var projects: [UInt64] = []
        var invoked: [String] = []
        var bindings: [String: String] = ["capture_task": "t", "capture_note": "n"]

        func focusCaptureTyped(_ text: String) { typed.append(text) }
        func focusCaptureDue(_ day: String?) { days.append(day) }
        func focusCaptureFilter(_ text: String) { filters.append(text) }
        func focusCaptureProject(_ token: UInt64) { projects.append(token) }
        func invoke(_ id: String) { invoked.append(id) }
        func binding(for command: String) -> String? { bindings[command] }
    }

    static func view(
        mode: CaptureModeFfi = .task, text: String = "Please leave comments by Wednesday",
        open: Bool = false, filter: String = ""
    ) -> CaptureViewFfi {
        CaptureViewFfi(
            mode: mode, from: "From Lane Example \u{b7} API draft \u{b7} today 15:22",
            field: mode == .task ? "Task" : "Note", text: text, subjectKey: "alt+s",
            hasDue: mode == .task, due: "2026-09-30", dueLabel: "Wednesday, 30 September 2026",
            picks: [
                CapturePickFfi(words: "Today", day: "2026-09-26"),
                CapturePickFfi(words: "Wed", day: "2026-09-30"),
                CapturePickFfi(words: "None", day: nil),
            ],
            projectTitle: "Project \u{b7} suggested: the subject names Harbor",
            project: "Harbor", projectNote: "Projects/Harbor.md", projectKey: "cmd+p",
            projectsOpen: open, filter: filter,
            projects: [
                CaptureProjectFfi(token: 1, name: "Harbor", note: "Projects/Harbor.md", open: "12 open", chosen: true),
                CaptureProjectFfi(token: 2, name: "Inbox", note: "Tasks.md (no project)", open: "21 open", chosen: false),
            ],
            previewTitle: "This exact line will be appended",
            preview: "- [ ] \(text) [\u{2709}](postio://message/7) \u{1f4c5} 2026-09-30",
            footnote: "Plain markdown, written on this computer.",
            button: mode == .task ? "Add task" : "Add note", buttonKey: "cmd+Return", error: nil)
    }

    static func model() -> (CaptureModel, Engine) {
        let engine = Engine()
        return (CaptureModel(engine: engine), engine)
    }

    @Test func tOpensCaptureWithTheSentenceVerbatimAndItsKeys() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusOpenCapture(view: Self.view())) == .open)
        #expect(model.isOpen)
        #expect(model.text == "Please leave comments by Wednesday")
        #expect(model.view?.preview.hasSuffix("2026-09-30") == true, "the exact line")
        #expect(model.subjectCap == "\u{2325}S")
        #expect(model.projectCap == "\u{2318}P")
        #expect(model.buttonCap == "\u{2318}\u{21a9}")
        #expect(model.taskCap == "t")
        #expect(model.noteCap == "n")
        #expect(model.apply(.focusCapture(view: Self.view(open: true))) == .redraw)
        #expect(model.view?.projectsOpen == true)
    }

    @Test func theFieldsFollowTheControllerAndSayWhatIsTyped() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenCapture(view: Self.view()))
        model.typed("Please leave comments by Wednesday")
        #expect(engine.typed.isEmpty, "the controller's own words are not said back")
        model.typed("Leave comments")
        #expect(engine.typed == ["Leave comments"])
        // ⌥S: the subject comes back as the text.
        model.apply(.focusCapture(view: Self.view(text: "API draft v3")))
        #expect(model.text == "API draft v3")

        model.apply(.focusCapture(view: Self.view(open: true)))
        model.filtered("harb")
        model.filtered("harb")
        #expect(engine.filters == ["harb"])
    }

    @Test func picksProjectsAndButtonsAreSaid() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenCapture(view: Self.view()))
        model.pick(CapturePickFfi(words: "Wed", day: "2026-09-30"))
        model.pick(CapturePickFfi(words: "None", day: nil))
        model.choose(2)
        model.choose(mode: .note)
        model.useSubject()
        model.changeProject()
        model.write()
        model.cancel()
        #expect(engine.days == ["2026-09-30", nil])
        #expect(engine.projects == [2])
        #expect(engine.invoked == [
            "capture_note", "capture_use_subject", "capture_change_project", "capture_write", "back",
        ])
    }

    @Test func itClosesOnItsOwnCloseAndAToolkitCloseIsSaidOnce() {
        let (model, _) = Self.model()
        model.apply(.focusOpenCapture(view: Self.view()))
        #expect(model.apply(.focusCloseSurface(kind: .dialog)) == nil)
        #expect(model.apply(.focusCloseSurface(kind: .capture)) == .close)
        #expect(!model.isOpen)
        #expect(!model.closedByToolkit())
        model.apply(.focusOpenCapture(view: Self.view()))
        #expect(model.closedByToolkit())
        #expect(!model.closedByToolkit())
    }
}
