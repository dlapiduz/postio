import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The digest-this-sender sheet, as the controller's intents leave it
/// (specs/009-focus-macos T115, screen 24).
///
/// The controller keeps the draft rule -- who it matches, its schedule,
/// the preview it read -- and writes it. The Mac draws every
/// `FocusOpenRule`/`FocusRule` whole and hands back each change of a
/// control, the two links, Create, and Cancel as the controller's Back.
@MainActor
struct RuleSheetModelTests {
    final class Engine: RuleEngine {
        var queries: [String] = []
        var instead = 0
        var likeThis = 0
        var schedules: [RuleScheduleFfi] = []
        var created = 0
        var invoked: [String] = []

        func focusRuleQuery(_ text: String) { queries.append(text) }
        func focusRuleMatchInstead() { instead += 1 }
        func focusRuleLikeThis() { likeThis += 1 }
        func focusRuleSchedule(_ schedule: RuleScheduleFfi) { schedules.append(schedule) }
        func focusRuleCreate() { created += 1 }
        func invoke(_ id: String) { invoked.append(id) }
    }

    static func view(query: String? = nil, preview: Bool = true, error: String? = nil) -> RuleViewFfi {
        RuleViewFfi(
            heading: "Digest this sender", fromLabel: "From",
            from: query == nil ? "board@example.org" : nil, query: query,
            placeholder: "from:list@example.org or a search",
            matchInstead: query == nil ? "Match a list or a search instead\u{2026}" : nil,
            likeThis: nil, deliverLabel: "Deliver",
            schedule: RuleScheduleFfi(cadence: 1, weekday: 6, monthDay: 0, at: "09:00"),
            cadences: ["Daily", "Weekly", "Monthly"],
            weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
            note: "Mail from this sender with an invite, question or to-do still comes straight to the inbox.",
            create: "Create", createKey: "Return",
            previewHeading: preview ? "Would have caught 9 messages in the last 90 days" : nil,
            preview: preview
                ? [RulePreviewLineFfi(subject: "Meeting agenda", day: "Today"),
                   RulePreviewLineFfi(subject: "Pool closing", day: "Wed")]
                : [],
            more: preview ? "and 7 more" : nil, error: error)
    }

    static func model() -> (RuleSheetModel, Engine) {
        let engine = Engine()
        return (RuleSheetModel(engine: engine), engine)
    }

    @Test func dOpensTheSheetPrefilledWithTheSender() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusOpenRule(view: Self.view())) == .open)
        #expect(model.isOpen)
        #expect(model.view?.from == "board@example.org")
        #expect(model.view?.preview.map(\.subject) == ["Meeting agenda", "Pool closing"])
        #expect(model.createCap == "\u{21a9}" || model.createCap == "Return")
        #expect(model.apply(.focusRule(view: Self.view(preview: false))) == .redraw)
        #expect(model.view?.previewHeading == nil)
    }

    @Test func theScheduleIsSaidWholeOnEveryChange() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenRule(view: Self.view()))
        model.setCadence(2)
        model.setWeekday(0)
        model.setMonthDay(14)
        model.setTime("18:30")
        #expect(engine.schedules == [
            RuleScheduleFfi(cadence: 2, weekday: 6, monthDay: 0, at: "09:00"),
            RuleScheduleFfi(cadence: 1, weekday: 0, monthDay: 0, at: "09:00"),
            RuleScheduleFfi(cadence: 1, weekday: 6, monthDay: 14, at: "09:00"),
            RuleScheduleFfi(cadence: 1, weekday: 6, monthDay: 0, at: "18:30"),
        ])
        #expect(model.view?.schedule.cadence == 1, "the sheet shows what the controller says")
    }

    @Test func theQueryFieldFollowsTheControllerAndSaysWhatIsTyped() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenRule(view: Self.view()))
        model.matchInstead()
        #expect(engine.instead == 1)
        model.apply(.focusRule(view: Self.view(query: "from:board@example.org")))
        #expect(model.query == "from:board@example.org", "the field is set from the view")
        model.typed("from:board@example.org")
        #expect(engine.queries.isEmpty, "the controller's own words are not said back")
        model.typed("list:board")
        #expect(engine.queries == ["list:board"])
        model.apply(.focusRule(view: Self.view(query: "list:board")))
        #expect(model.query == "list:board")
    }

    @Test func createCancelAndCloses() {
        let (model, engine) = Self.model()
        model.apply(.focusOpenRule(view: Self.view()))
        model.create()
        model.cancel()
        #expect(engine.created == 1)
        #expect(engine.invoked == ["back"])
        #expect(model.isOpen, "closed only when the controller says")
        #expect(model.apply(.focusCloseSurface(kind: .capture)) == nil)
        #expect(model.apply(.focusCloseSurface(kind: .dialog)) == .close)
        #expect(!model.isOpen)
        #expect(!model.closedByToolkit())
        model.apply(.focusOpenRule(view: Self.view()))
        #expect(model.closedByToolkit())
        #expect(!model.closedByToolkit())
    }

    @Test func aCloseOfAnotherDialogIsNotTheSheets() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusCloseSurface(kind: .dialog)) == nil)
    }
}
