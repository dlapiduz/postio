import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The timeline's drag and the filter popovers' model (specs/010-focus-search
/// T082; design §3.3, §3.6, screens 08 and 09).
///
/// What months a range means, and what the query becomes, are the
/// controller's (`crates/postio-focus/src/results.rs`). The Mac reports a
/// drag as the bars it began and ended over -- once, when the pointer lets
/// go, so a drag is one search and not one per bar it crosses -- and what
/// happened in a popover, by the token of the row it happened to.
@MainActor
struct TimelineTests {
    /// Twelve bars across 120 points: ten points a bar.
    static let width = 120.0

    @Test func aDragFromBar3ToBar5ReportsThreeToFiveOnceOnReleaseNeverDuring() {
        var reports: [[UInt32]] = []
        let drag = TimelineDrag(bars: 12) { reports.append([$0, $1]) }

        drag.changed(start: 35, at: 35, width: Self.width)
        drag.changed(start: 35, at: 45, width: Self.width)
        drag.changed(start: 35, at: 55, width: Self.width)
        #expect(reports.isEmpty, "nothing is asked while the pointer moves")
        #expect(drag.band == 3...5, "the band follows the pointer, drawn here")

        drag.ended(start: 35, at: 55, width: Self.width)
        #expect(reports == [[3, 5]])
        #expect(drag.band == nil)
    }

    @Test func aDragRightToLeftReportsTheSameMonths() {
        var reports: [[UInt32]] = []
        let drag = TimelineDrag(bars: 12) { reports.append([$0, $1]) }

        drag.changed(start: 58, at: 44, width: Self.width)
        drag.changed(start: 58, at: 31, width: Self.width)
        drag.ended(start: 58, at: 31, width: Self.width)
        #expect(reports == [[3, 5]])
    }

    @Test func aClickOnOneBarReportsThatMonthAlone() {
        var reports: [[UInt32]] = []
        let drag = TimelineDrag(bars: 12) { reports.append([$0, $1]) }

        drag.changed(start: 72, at: 72, width: Self.width)
        drag.ended(start: 72, at: 73, width: Self.width)
        #expect(reports == [[7, 7]])
    }

    @Test func aDragPastEitherEndStopsAtTheEndBar() {
        var reports: [[UInt32]] = []
        let drag = TimelineDrag(bars: 12) { reports.append([$0, $1]) }

        drag.ended(start: -10, at: 500, width: Self.width)
        #expect(reports == [[0, 11]])
    }

    // MARK: the popover's model

    final class Engine: FilterPopoverEngine {
        var calls: [String] = []
        func focusSearchPopover(_ kind: FilterKindFfi) { calls.append("open \(kind)") }
        func focusSearchPopoverToggle(_ token: UInt64, exclude: Bool) {
            calls.append("toggle \(token) \(exclude)")
        }
        func focusSearchPopoverFilter(_ text: String) { calls.append("filter \(text)") }
        func focusSearchPopoverDone(_ apply: Bool) { calls.append("done \(apply)") }
        func focusSearchDateWords(_ text: String) { calls.append("words \(text)") }
        func focusSearchDatePreset(_ token: UInt64) { calls.append("preset \(token)") }
        func focusSearchMonths(_ first: UInt32, _ last: UInt32) { calls.append("months \(first) \(last)") }
    }

    static func from() -> PopoverViewFfi {
        PopoverViewFfi(
            kind: .from, placeholder: "Filter people in these results", filter: "",
            rows: [
                PopoverRowFfi(
                    token: 0, title: "Ada Moreno", detail: "ada@example.com", initials: "AM", color: nil,
                    count: 21, share: 1, checked: false, excluded: false),
                PopoverRowFfi(
                    token: 1, title: "Tomás Reyes", detail: "tomas@example.com", initials: "TR", color: nil,
                    count: 9, share: 0.43, checked: false, excluded: false),
            ],
            hints: [], presets: [], words: "", parsed: nil, wordsHint: "", months: [], result: nil, range: nil)
    }

    @Test func thePopoverIsWhatTheControllerLastSaidAndClosesWhenItSaysNone() {
        let engine = Engine()
        let model = FilterPopoverModel(engine: engine)
        #expect(model.apply(.focusPopover(view: Self.from())) == .open(.from))
        #expect(model.view?.rows.count == 2)
        #expect(model.highlight == 0, "the first row takes Space")

        #expect(model.apply(.focusPopover(view: nil)) == .close)
        #expect(model.view == nil)
        // A close the controller made is not reported back.
        model.closedByToolkit()
        #expect(engine.calls.isEmpty)
    }

    @Test func spaceAndAltClickReportTheRowAndEscAClickAwayReportsOnce() {
        let engine = Engine()
        let model = FilterPopoverModel(engine: engine)
        model.apply(.focusPopover(view: Self.from()))

        model.moveHighlight(by: 1)
        model.toggleHighlighted()
        model.toggle(token: 0, exclude: true)
        model.closedByToolkit()
        #expect(engine.calls == ["toggle 1 false", "toggle 0 true", "done false"])
    }
}
