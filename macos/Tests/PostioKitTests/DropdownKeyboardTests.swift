import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The search dropdown's keys on the Mac (specs/010-focus-search T052;
/// design §2 "Keys in the dropdown").
///
/// The controller decides what the dropdown shows and what a row runs
/// (`crates/postio-focus`, the dropdown section of `tests/bar.rs`). The
/// one thing the Mac decides is where the arrows rest (009 FR-004): ↓/↑
/// walk the rows that run, skipping headers and the cheat sheet; a new
/// view keeps the highlight on the same row while it is still drawn, else
/// takes the view's default; a run that moved it (`select`) wins. Every
/// key is handed down: ↩ runs the highlighted row's token, ⌘↩ is Show
/// all, Tab is the bar's, ⌥⌫ on a recent forgets it, Esc is Back.
@MainActor
struct DropdownKeyboardTests {
    final class Engine: CommandBarEngine {
        var ran: [UInt64] = []
        var tabs = 0
        var invoked: [String] = []
        var forgotten: [UInt64] = []
        var shownAll = 0
        var highlightedTokens: [UInt64] = []
        var excluded: [UInt64] = []

        func focusBarTyped(_ text: String) {}
        func focusBarRun(_ token: UInt64) { ran.append(token) }
        func focusBarTab() -> Bool {
            tabs += 1
            return true
        }
        func invoke(_ id: String) { invoked.append(id) }
        func binding(for command: String) -> String? { nil }
        func focusSearchForget(_ token: UInt64) { forgotten.append(token) }
        func focusSearchShowAll() { shownAll += 1 }
        func focusSearchHighlighted(_ token: UInt64) { highlightedTokens.append(token) }
        func focusSearchExclude(_ token: UInt64) { excluded.append(token) }
    }

    static func run(_ text: String) -> RunFfi { RunFfi(text: text, highlighted: false, style: .plain) }

    static func row(_ token: UInt64, _ kind: DropdownRowKindFfi, _ title: String, selectable: Bool = true)
        -> DropdownRowFfi
    {
        DropdownRowFfi(
            token: token, kind: kind, title: [run(title)], detail: [], folder: nil, right: nil,
            key: nil, selectable: selectable, initials: kind == .person ? "AM" : nil)
    }

    static func section(_ title: String, _ rows: [DropdownRowFfi]) -> DropdownSectionFfi {
        DropdownSectionFfi(title: title, note: nil, noteKey: nil, rows: rows, pills: [])
    }

    /// Screen 01: three recents, the saved pills, the cheat sheet.
    static func empty(recents: [UInt64] = [11, 12, 13], highlight: UInt64? = 11, select: UInt64? = nil)
        -> UiEvent
    {
        .focusDropdown(
            view: DropdownViewFfi(
                state: .empty, ghost: nil, understood: [],
                sections: [
                    section("Recent", recents.map { row($0, .recent, "recent \($0)") }),
                    DropdownSectionFfi(
                        title: "Saved searches", note: nil, noteKey: nil, rows: [],
                        pills: [PillFfi(token: 20, op: nil, label: "Atlas", count: "38", fresh: nil, key: "alt+2")]),
                    section(
                        "Search by",
                        [row(30, .cheatSheet, "from:", selectable: false),
                         row(31, .example, "Or just type it:", selectable: false)]),
                ],
                highlight: highlight, select: select, footerHints: [], footerCount: nil))
    }

    /// Screen 03: four hits and Show all, focused by default.
    static func words(highlight: UInt64? = 49, select: UInt64? = nil) -> UiEvent {
        .focusDropdown(
            view: DropdownViewFfi(
                state: .words, ghost: nil, understood: [],
                sections: [
                    section("Top hits", (41...44).map { row($0, .hit, "hit \($0)") }),
                    section("", [row(49, .showAll, "Show all 48 results")]),
                ],
                highlight: highlight, select: select,
                footerHints: [KeyHintFfi(key: "cmd+Return", label: "all results")],
                footerCount: "48 matches · 38 ms"))
    }

    /// Screen 04: two people, a label, then the latest from the first.
    static func people() -> UiEvent {
        .focusDropdown(
            view: DropdownViewFfi(
                state: .operator, ghost: nil, understood: [],
                sections: [
                    section("People", [row(51, .person, "Ada Moreno"), row(52, .person, "Ben Adeyemi"),
                                       row(53, .label, "Atlas")]),
                    section("Latest from Ada Moreno", [row(61, .hit, "hit 61")]),
                ],
                highlight: 51, select: nil,
                footerHints: [KeyHintFfi(key: "alt+Return", label: "exclude (-from:)")],
                footerCount: "Contacts and everyone you have mail with"))
    }

    /// Screen 02: the ghost after "at", the word focused.
    static func prefix() -> UiEvent {
        .focusDropdown(
            view: DropdownViewFfi(
                state: .prefix, ghost: "las", understood: [],
                sections: [section("Suggestions", [row(71, .word, "atlas")])],
                highlight: 71, select: nil, footerHints: [], footerCount: nil))
    }

    static func model() -> (CommandBarModel, Engine) {
        let engine = Engine()
        let model = CommandBarModel(engine: engine)
        model.apply(.focusOpenBar(mode: .search, text: "", select: nil))
        return (model, engine)
    }

    @Test func theArrowsWalkTheRowsThatRunAndSkipHeadersAndTheCheatSheet() {
        let (model, engine) = Self.model()
        #expect(model.apply(Self.empty()) == .lines)
        #expect(model.showsDropdown)
        #expect(model.dropdown.highlighted == 11, "the view's default")
        model.move(by: 1)
        model.move(by: 1)
        #expect(model.dropdown.highlighted == 13)
        model.move(by: 1)
        #expect(model.dropdown.highlighted == 13, "stops at the last row that runs")
        model.move(by: -5)
        #expect(model.dropdown.highlighted == 11)
        #expect(engine.highlightedTokens == [12, 13, 11], "where the arrows rest is said")
    }

    @Test func returnRunsTheHighlightedRowsToken() {
        let (model, engine) = Self.model()
        model.apply(Self.words())
        model.move(by: -1)
        model.runHighlighted()
        #expect(engine.ran == [44])
    }

    @Test func commandReturnIsShowAllAndTabIsTheBars() {
        let (model, engine) = Self.model()
        model.apply(Self.words())
        model.showAll()
        #expect(engine.shownAll == 1)
        #expect(model.tab())
        #expect(engine.tabs == 1)
    }

    @Test func optionDeleteForgetsARecentAndNothingElse() {
        let (model, engine) = Self.model()
        model.apply(Self.empty())
        model.move(by: 1)
        #expect(model.forgetHighlighted())
        #expect(engine.forgotten == [12])
        model.apply(Self.words())
        #expect(!model.forgetHighlighted(), "a hit is not a recent: the field's own ⌥⌫")
        #expect(engine.forgotten == [12])
    }

    @Test func escapeIsBackWhichClosesAndSendsTheKeyboardHome() {
        let (model, engine) = Self.model()
        model.apply(Self.empty())
        model.back()
        #expect(engine.invoked == ["back"])
        #expect(model.apply(.focusCloseSurface(kind: .bar)) == .close)
        #expect(!model.showsDropdown)
        #expect(model.dropdown.sections.isEmpty)
    }

    @Test func aNewViewKeepsTheHighlightOnTheSameRowElseTakesTheDefault() {
        let (model, _) = Self.model()
        model.apply(Self.empty())
        model.move(by: 2)
        #expect(model.dropdown.highlighted == 13)
        // The recents read again, the same three: the arrows stay.
        model.apply(Self.empty())
        #expect(model.dropdown.highlighted == 13)
        // The highlighted one forgotten: the view's default.
        model.apply(Self.empty(recents: [11, 12], highlight: 11))
        #expect(model.dropdown.highlighted == 11)
        // A run moved it: taken, whatever was highlighted.
        model.apply(Self.words())
        #expect(model.dropdown.highlighted == 49)
        model.apply(Self.words(select: 41))
        #expect(model.dropdown.highlighted == 41)
    }

    @Test func linesAfterTheDropdownPutTheLinesBack() {
        let (model, _) = Self.model()
        model.apply(Self.words())
        model.apply(
            .focusBarLines(
                view: BarViewFfi(
                    heading: nil, echo: nil, chips: [], editing: nil,
                    lines: [CommandBarModelTests.line(.command, 7, "Archive")], highlight: nil,
                    saved: [])))
        #expect(!model.showsDropdown)
        model.move(by: 1)
        #expect(model.highlighted == 7, "the arrows walk the lines again")
    }

    @Test func optionReturnExcludesTheHighlightedPersonLabelOrFolder() {
        let (model, engine) = Self.model()
        model.apply(Self.people())
        #expect(model.dropdown.state == .operator)
        #expect(model.dropdown.highlightedRow?.initials == "AM", "the avatar's letters")
        model.move(by: 1)
        #expect(model.excludeHighlighted())
        #expect(engine.excluded == [52], "Ben, where the arrows rest")
        model.move(by: 1)
        #expect(model.excludeHighlighted())
        #expect(engine.excluded == [52, 53], "a label too")
        model.move(by: 1)
        #expect(!model.excludeHighlighted(), "a message is not excluded: the field's own key")
        model.apply(Self.words())
        #expect(!model.excludeHighlighted())
        #expect(engine.excluded == [52, 53])
    }

    @Test func movingTheHighlightOntoAPersonSaysSoForTheirLatest() {
        let (model, engine) = Self.model()
        model.apply(Self.people())
        model.move(by: 1)
        #expect(engine.highlightedTokens == [52], "the controller asks for Ben's latest")
    }

    @Test func tabWithAGhostIsTheBarsAndTheGhostIsKept() {
        let (model, engine) = Self.model()
        model.apply(Self.prefix())
        #expect(model.dropdown.ghost == "las", "drawn after the caret")
        #expect(model.tab())
        #expect(engine.tabs == 1)
        model.apply(Self.words())
        #expect(model.dropdown.ghost == nil, "gone with the prefix state")
    }

    @Test func plainEnglishKeepsItsTiles() {
        let (model, _) = Self.model()
        model.apply(
            .focusDropdown(
                view: DropdownViewFfi(
                    state: .plainEnglish, ghost: nil,
                    understood: [
                        UnderstoodTileFfi(op: "", value: "invoices", origin: "from \u{2018}invoices\u{2019}"),
                        UnderstoodTileFfi(op: "after:", value: "2026-08-01", origin: "from \u{2018}last month\u{2019}"),
                    ],
                    sections: [Self.section("Results", [Self.row(81, .hit, "hit 81")])],
                    highlight: 81, select: nil, footerHints: [], footerCount: nil)))
        #expect(model.dropdown.understood.map(\.value) == ["invoices", "2026-08-01"])
        #expect(model.dropdown.understood.last?.origin == "from \u{2018}last month\u{2019}")
    }
}
