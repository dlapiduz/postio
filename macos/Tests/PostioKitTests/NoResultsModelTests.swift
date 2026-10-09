import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The no-results page's model (specs/010-focus-search T106; design §3.10,
/// screen 13). The controller says what the page draws and when it goes;
/// a click on a way out runs it as its number would.
@MainActor
struct NoResultsModelTests {
    final class Engine: NoResultsEngine {
        var invoked: [String] = []
        func invoke(_ id: String) { invoked.append(id) }
    }

    nonisolated static func view(counting: Bool = false) -> NoResultsViewFfi {
        NoResultsViewFfi(
            title: "Nothing matches all four filters",
            body: "Each line below loosens one filter and shows how many conversations you would get.",
            relaxations: counting
                ? []
                : [
                    RelaxationFfi(
                        number: 1, key: "1", label: "Remove “before March”",
                        query: "from:ada has:attachment subject:\"budget v4\"", count: "4 conversations",
                        focused: true),
                    RelaxationFfi(
                        number: 2, key: "2", label: "Anyone, not just Ada Moreno",
                        query: "has:attachment before:2026-03-01 subject:\"budget v4\"", count: "3 conversations",
                        focused: false),
                ],
            counting: counting ? "Counting looser searches…" : nil,
            searched: "Searched all 18,204 messages on this Mac.")
    }

    @Test func itOpensRedrawsAndClosesAsTheControllerSays() {
        let model = NoResultsModel(engine: Engine())
        #expect(model.apply(.focusRelaxations(view: Self.view(counting: true))) == .open)
        #expect(model.view?.counting == "Counting looser searches…")
        #expect(model.apply(.focusRelaxations(view: Self.view())) == .redraw)
        #expect(model.view?.relaxations.count == 2)
        #expect(model.apply(.focusRelaxations(view: nil)) == .close)
        #expect(!model.isOpen)
        #expect(model.apply(.focusRelaxations(view: nil)) == nil, "closed once")
        #expect(model.apply(.focusLeaveResults) == nil, "nothing to close")
    }

    @Test func leavingTheResultsTakesThePageWithThem() {
        let model = NoResultsModel(engine: Engine())
        model.apply(.focusRelaxations(view: Self.view()))
        #expect(model.apply(.focusLeaveResults) == .close)
        #expect(!model.isOpen)
    }

    @Test func aClickOnAWayOutRunsItsNumber() {
        let engine = Engine()
        let model = NoResultsModel(engine: engine)
        model.apply(.focusRelaxations(view: Self.view()))
        model.pick(2)
        model.pick(5)
        #expect(engine.invoked == ["pick_relaxation_2"], "only the numbers that exist")
    }

    @Test func aWayOutsCapIsItsKey() {
        let model = NoResultsModel(engine: Engine())
        model.apply(.focusRelaxations(view: Self.view()))
        #expect(model.view.map { NoResultsView.cap(for: $0.relaxations[0]) } == "1")
    }
}
