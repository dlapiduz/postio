import Foundation
import PostioFFI
import Testing

@testable import PostioKit

@MainActor
struct CommandBarModelTests {
    final class Engine: CommandBarEngine {
        var typed: [String] = []
        var ran: [UInt64] = []
        var tabs = 0
        var tabAnswer = true
        var invoked: [String] = []
        var bindings: [String: String] = [
            "archive": "a",
            "go_to_archive": "g a",
            "command_palette": "cmd+k",
            "saved_search_1": "alt+1",
            "saved_search_2": "alt+2",
        ]

        func focusBarTyped(_ text: String) { typed.append(text) }
        func focusBarRun(_ token: UInt64) { ran.append(token) }
        func focusBarTab() -> Bool {
            tabs += 1
            return tabAnswer
        }
        func invoke(_ id: String) { invoked.append(id) }
        func binding(for command: String) -> String? { bindings[command] }
        func focusSearchHighlighted(_ token: UInt64) {}
        func focusSearchForget(_ token: UInt64) {}
        func focusSearchShowAll() {}
        func focusSearchExclude(_ token: UInt64) {}
    }

    static func line(
        _ kind: BarLineKindFfi, _ token: UInt64, _ title: String, detail: String? = nil,
        key: String? = nil, command: String? = nil, selectable: Bool = true
    ) -> BarLineFfi {
        BarLineFfi(
            kind: kind, token: token, title: title, detail: detail, key: key, command: command,
            selectable: selectable)
    }

    static let archLines: [BarLineFfi] = [
        line(.heading, 1, "Commands", selectable: false),
        line(.command, 2, "Archive", detail: "the focused message", key: "a", command: "archive"),
        line(.command, 3, "Archive everything read, older than a week"),
        line(.heading, 4, "Go to", selectable: false),
        line(.command, 5, "Go to Archive", key: "g a", command: "go_to_archive"),
        line(.hint, 6, "Commands only", key: ">", selectable: false),
        line(.correspondent, 7, "Ada Moreno", detail: "ada@example.com"),
    ]

    static func lines(
        _ lines: [BarLineFfi] = archLines, echo: String? = nil, highlight: UInt64? = nil,
        saved: [String] = []
    ) -> UiEvent {
        .focusBarLines(view: BarViewFfi(echo: echo, lines: lines, highlight: highlight, saved: saved))
    }

    static func opened(_ text: String = "", mode: BarModeFfi = .search, select: BarSelectFfi? = nil)
        -> UiEvent
    {
        .focusOpenBar(mode: mode, text: text, select: select)
    }

    static func model() -> (CommandBarModel, Engine) {
        let engine = Engine()
        return (CommandBarModel(engine: engine), engine)
    }

    @Test func theLinesBecomeOneLineRowsWithTheUsersKeycaps() {
        let (model, engine) = Self.model()
        model.apply(Self.opened(">arch", mode: .commands))
        engine.bindings["archive"] = "e"
        model.apply(Self.lines())

        #expect(model.rows.map(\.title) == Self.archLines.map(\.title))
        #expect(model.rows.map(\.kind) == Self.archLines.map(\.kind))
        let caps = model.rows.map(\.cap)
        #expect(caps[1] == "e", "the user's binding, spelled as a cap")
        #expect(caps[2] == nil, "a command with no key draws no cap")
        #expect(caps[4] == "g a", "a sequence is one cap, its presses spaced")
        #expect(caps[5] == ">", "a hint's prefix is its cap")
        #expect(model.rows[1].detail == "the focused message")
        #expect(model.rows[6].detail == "ada@example.com")
    }

    @Test func aChordIsSpelledWithTheMacsGlyphs() {
        let (model, _) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines([Self.line(.command, 9, "Command bar", key: "mod+k", command: "command_palette")]))
        #expect(model.rows.first?.cap == "⌘K")
    }

    @Test func theEchoIsKept() {
        let (model, _) = Self.model()
        model.apply(Self.opened(">arch", mode: .commands))
        model.apply(Self.lines(echo: "You typed “>arch”"))
        #expect(model.echo == "You typed “>arch”")
    }

    @Test func theSavedSearchesCarryTheirKeys() {
        let (model, _) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines(saved: ["Waiting on reply", "Atlas", "Receipts this month"]))
        #expect(model.saved.map(\.name) == ["Waiting on reply", "Atlas", "Receipts this month"])
        #expect(model.saved.map(\.cap) == ["⌥1", "⌥2", nil])
    }

    @Test func linesBeforeTheBarOpensOrAfterItClosesDrawNothing() {
        let (model, _) = Self.model()
        #expect(model.apply(Self.lines()) == nil)
        #expect(model.rows.isEmpty)
        model.apply(Self.opened())
        model.apply(Self.lines())
        #expect(model.apply(.focusCloseSurface(kind: .bar)) == .close)
        #expect(!model.isOpen)
        #expect(model.rows.isEmpty, "a closed bar keeps nothing it showed")
    }

    @Test func otherEventsAreNotTheBars() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusCloseSurface(kind: .message)) == nil)
        #expect(model.apply(.focusKeyboardHome) == nil)
    }

    @Test func openingPutsTheWordsInTheFieldWithTheChipSelected() {
        let (model, _) = Self.model()
        #expect(
            model.apply(Self.opened(">", mode: .commands))
                == .open(text: ">", selection: NSRange(location: 1, length: 0)))
        #expect(model.isOpen)
        #expect(model.mode == .commands)
        // Characters, not UTF-16 units: the controller counts `char`s.
        let change = model.apply(
            Self.opened("subject:é invoice", select: BarSelectFfi(start: 10, end: 17)))
        #expect(change == .open(text: "subject:é invoice", selection: NSRange(location: 10, length: 7)))
        let emoji = model.apply(Self.opened("👍 x", select: BarSelectFfi(start: 2, end: 3)))
        #expect(emoji == .open(text: "👍 x", selection: NSRange(location: 3, length: 1)))
    }

    @Test func typingForwardsTheWordsToTheController() {
        let (model, engine) = Self.model()
        model.apply(Self.opened(">", mode: .commands))
        model.typed(">")
        #expect(engine.typed.isEmpty, "the field echoing what the controller put there is not typing")
        model.typed(">a")
        model.typed(">ar")
        #expect(engine.typed == [">a", ">ar"])
    }

    @Test func typingWithTheBarClosedTellsTheControllerNothing() {
        let (model, engine) = Self.model()
        model.typed("tide")
        #expect(engine.typed.isEmpty)
    }

    @Test func theHighlightStartsOnTheFirstLineThatRuns() {
        let (model, _) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines())
        #expect(model.highlighted == 2, "the heading cannot be highlighted")
    }

    @Test func theArrowsSkipWhatCannotRunAndStopAtTheEnds() {
        let (model, _) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines())
        model.move(by: 1)
        #expect(model.highlighted == 3)
        model.move(by: 1)
        #expect(model.highlighted == 5, "past the Go to heading")
        model.move(by: 1)
        #expect(model.highlighted == 7, "past the hint")
        model.move(by: 1)
        #expect(model.highlighted == 7, "the last line stays")
        model.move(by: -10)
        #expect(model.highlighted == 2)
    }

    @Test func theControllersHighlightWinsAndOtherwiseItStays() {
        let (model, _) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines())
        model.move(by: 1)
        model.apply(Self.lines())
        #expect(model.highlighted == 3)
        model.apply(Self.lines(highlight: 7))
        #expect(model.highlighted == 7)
        model.apply(Self.lines([Self.line(.command, 20, "Compose", command: "compose")]))
        #expect(model.highlighted == 20)
    }

    @Test func returnRunsTheHighlightedLinesToken() {
        let (model, engine) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines())
        model.move(by: 1)
        model.runHighlighted()
        #expect(engine.ran == [3])
        model.run(5)
        #expect(engine.ran == [3, 5], "a click runs its own line")
    }

    @Test func returnWithNothingToRunRunsNothing() {
        let (model, engine) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines([Self.line(.hint, 1, "Search mail", selectable: false)]))
        #expect(model.highlighted == nil)
        model.runHighlighted()
        #expect(engine.ran.isEmpty)
    }

    @Test func escapeClosesThroughTheControllersBack() {
        let (model, engine) = Self.model()
        model.apply(Self.opened())
        model.back()
        #expect(engine.invoked == ["back"])
        #expect(model.isOpen, "the bar closes when the controller says so, not before")
        model.apply(.focusCloseSurface(kind: .bar))
        #expect(!model.isOpen)
    }

    @Test func tabIsTheBarsWhileThereIsAChipAndTheToolkitsOtherwise() {
        let (model, engine) = Self.model()
        model.apply(Self.opened("from:ada invoice"))
        #expect(model.tab(), "into the chips: the key is used")
        engine.tabAnswer = false
        #expect(!model.tab(), "no chip: the toolkit keeps Tab")
        #expect(engine.tabs == 2)
    }

    @Test func tabWithTheBarClosedIsTheToolkits() {
        let (model, engine) = Self.model()
        #expect(!model.tab())
        #expect(engine.tabs == 0)
    }

    @Test func theToolkitClosingTheBarIsReportedOnce() {
        let (model, _) = Self.model()
        #expect(!model.closedByToolkit(), "nothing open, nothing to report")
        model.apply(Self.opened())
        model.apply(Self.lines())
        #expect(model.closedByToolkit(), "a click outside: say it closed")
        #expect(!model.isOpen)
        #expect(model.rows.isEmpty)
        #expect(!model.closedByToolkit())
    }

    @Test func aSavedSearchRunsItsCommand() {
        let (model, engine) = Self.model()
        model.apply(Self.opened())
        model.apply(Self.lines(saved: ["Waiting on reply", "Atlas"]))
        model.runSaved(1)
        #expect(engine.invoked == ["saved_search_2"], "what ⌥2 runs, so a click and the key agree")
        model.runSaved(7)
        #expect(engine.invoked == ["saved_search_2"], "nothing pinned there")
    }

    @Test func theCommandsTheBarRunsByNameAreRealCommands() {
        // A literal that no longer matches the registry is a key that does nothing, silently.
        let known = Set(PostioRegistry.commands.map(\.id))
        let named = [BarCommand.search, BarCommand.palette, BarCommand.back, BarCommand.saveSearch,
                     BarCommand.folders] + CommandBarModel.savedCommands
        for id in named {
            #expect(known.contains(id), "`\(id)` is not a command")
        }
    }
}
