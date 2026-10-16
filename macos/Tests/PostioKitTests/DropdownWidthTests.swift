import AppKit
import SwiftUI
import Testing

@testable import PostioKit

/// The dropdown is never wider than its panel (specs/010-focus-search).
///
/// The panel takes the search field's width, which is less than the design's
/// 860 in a smaller window. A row that asks for more than that widens the
/// whole dropdown, and SwiftUI centres the wider content in the panel: every
/// line loses its start and its end, the hits and the footer included.
@MainActor
struct DropdownWidthTests {
    private func pills(_ count: Int) -> [DropdownModel.Pill] {
        (0..<count).map {
            DropdownModel.Pill(
                id: UInt64($0), op: "from:", label: "Sender Number \($0) at Example", count: "4",
                fresh: nil, cap: nil)
        }
    }

    private func width<V: View>(of view: V, in room: CGFloat) -> CGFloat {
        NSHostingController(rootView: view).sizeThatFits(in: CGSize(width: room, height: 200)).width
    }

    @Test func moreSendersThanFitDoNotWidenTheRow() {
        #expect(width(of: PillsRow(pills: pills(6), run: { _ in }), in: 500) <= 500)
    }

    @Test func theFirstPillsAreTheOnesShown() {
        // Tab adds the first filter, so the first pill has to be on screen.
        let shown = PillsRow.fitting(widths: [200, 200, 200], spacing: 8, room: 420)
        #expect(shown == 2)
    }

    @Test func pillsThatFitAreAllShown() {
        #expect(PillsRow.fitting(widths: [100, 100], spacing: 8, room: 500) == 2)
    }
}
