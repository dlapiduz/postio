import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Filtered, as the controller's intents leave it (specs/009-focus-macos
/// T113, screen 21).
///
/// The controller (`crates/postio-focus/src/filtered.rs`) keeps which tab
/// is showing, which row has the keyboard, the pages read and what each
/// key does. The Mac draws `FocusFiltered` whole in the list's place and
/// hands back what the pointer did: a row clicked, a tab or a button
/// clicked, the end of the rows reached.
@MainActor
struct FilteredModelTests {
    final class Engine: FilteredEngine {
        var pointed: [UInt32] = []
        var more = 0
        var invoked: [String] = []
        var bindings: [String: String] = [:]

        func focusFilteredPoint(_ index: UInt32) { pointed.append(index) }
        func focusFilteredMore() { more += 1 }
        func invoke(_ id: String) { invoked.append(id) }
        func binding(for command: String) -> String? { bindings[command] }
    }

    static func line(_ message: Int64, _ sender: String, pill: String, heading: String? = nil) -> FilteredLineFfi {
        FilteredLineFfi(
            message: message, sender: sender, subject: "Subject \(message)", preview: "First line",
            pill: pill, time: "16:0\(message % 10)", heading: heading)
    }

    static func view(
        rows: [FilteredLineFfi]? = nil, focused: UInt32? = 0, more: Bool = false, on tab: Int = 0
    ) -> FilteredViewFfi {
        let names = ["All", "Spam", "Promotions", "Notifications", "Receipts", "Shipping", "Social"]
        return FilteredViewFfi(
            title: "Filtered",
            subtitle: "Archived automatically \u{b7} newest first",
            note: "Nothing here is deleted automatically \u{b7} nothing here ever reached the inbox",
            sweep: "Sweep the inbox\u{2026}", sweepKey: "S",
            restore: "Restore, never filter this sender", restoreKey: "R",
            tabs: names.enumerated().map { index, name in
                FilteredTabFfi(name: name, count: UInt32(10 - index), key: "\(index + 1)", on: index == tab)
            },
            rows: rows ?? [
                line(1, "Build Bot", pill: "notification \u{b7} CI", heading: "Today \u{b7} 3"),
                line(2, "Shop Example", pill: "promotion"),
                line(3, "Parcel Example", pill: "shipping"),
            ],
            focused: focused, more: more,
            footer: [
                FocusHintFfi(key: "R", label: "restore + never filter sender"),
                FocusHintFfi(key: "1\u{2013}7", label: "reason tabs"),
            ])
    }

    static func model() -> (FilteredModel, Engine) {
        let engine = Engine()
        return (FilteredModel(engine: engine), engine)
    }

    @Test func gFShowsFilteredAndItsViewDrawsTheTabsRowsAndNote() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusShowFiltered) == .open)
        #expect(model.isOpen)
        #expect(model.rows.isEmpty, "nothing is drawn before the view lands")

        #expect(model.apply(.focusFiltered(view: Self.view())) == .redraw)
        #expect(model.title == "Filtered")
        #expect(model.note.hasPrefix("Nothing here is deleted automatically"), "C4")
        #expect(model.tabs.map(\.name) == ["All", "Spam", "Promotions", "Notifications", "Receipts", "Shipping", "Social"])
        #expect(model.tabs.map(\.count) == [10, 9, 8, 7, 6, 5, 4])
        #expect(model.tabs.map(\.cap) == ["1", "2", "3", "4", "5", "6", "7"])
        #expect(model.tabs.first?.on == true)
        #expect(model.rows.map(\.pill) == ["notification \u{b7} CI", "promotion", "shipping"])
        #expect(model.rows.first?.heading == "Today \u{b7} 3")
        #expect(model.restoreCap == "R")
        #expect(model.footer.map(\.label) == ["restore + never filter sender", "reason tabs"])
        #expect(model.focused == 0)
    }

    @Test func aViewWithoutShowFilteredOpensItToo() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusFiltered(view: Self.view())) == .open)
        #expect(model.isOpen)
    }

    @Test func theFocusMovesOnlyTheFocus() {
        let (model, _) = Self.model()
        model.apply(.focusFiltered(view: Self.view()))
        let rows = model.rows
        #expect(model.apply(.focusFilteredFocus(index: 2)) == .focus)
        #expect(model.focused == 2)
        #expect(model.rows == rows)
    }

    @Test func thePointerIsToldAndMovesNothingItself() {
        let (model, engine) = Self.model()
        model.apply(.focusFiltered(view: Self.view()))
        model.point(2)
        #expect(engine.pointed == [2])
        #expect(model.focused == 0, "the focus moves when the controller says so")

        model.open(1)
        #expect(engine.pointed == [2, 1])
        #expect(engine.invoked == ["open_message"])
    }

    @Test func theTabsAndButtonsAreTheirCommands() {
        let (model, engine) = Self.model()
        model.apply(.focusFiltered(view: Self.view()))
        model.chooseTab(3)
        model.restore()
        model.sweep()
        model.back()
        #expect(engine.invoked == ["filtered_tab_3", "restore_filtered", "sweep_inbox", "back"])
    }

    @Test func theEndOfTheRowsAsksForMoreOncePerPage() {
        let (model, engine) = Self.model()
        model.apply(.focusFiltered(view: Self.view(more: true)))
        model.reachedEnd()
        model.reachedEnd()
        #expect(engine.more == 1, "one read per page")

        let longer = Self.view(rows: (1...5).map { Self.line($0, "Sender \($0)", pill: "spam") }, more: true)
        model.apply(.focusFiltered(view: longer))
        model.reachedEnd()
        #expect(engine.more == 2, "the next page asks again")

        model.apply(.focusFiltered(view: Self.view(more: false)))
        model.reachedEnd()
        #expect(engine.more == 2, "nothing more to read")
    }

    @Test func itClosesOnlyOnItsOwnClose() {
        let (model, _) = Self.model()
        model.apply(.focusFiltered(view: Self.view()))
        #expect(model.apply(.focusCloseSurface(kind: .picker)) == nil)
        #expect(model.isOpen)
        #expect(model.apply(.focusCloseSurface(kind: .filtered)) == .close)
        #expect(!model.isOpen)
        #expect(model.rows.isEmpty)
    }

    @Test func otherEventsAreNotFiltereds() {
        let (model, _) = Self.model()
        #expect(model.apply(.focusKeyboardHome) == nil)
        #expect(model.apply(.focusFilteredFocus(index: 1)) == nil, "no Filtered up to focus in")
    }
}
