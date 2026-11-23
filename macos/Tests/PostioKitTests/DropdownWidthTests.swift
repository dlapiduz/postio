import AppKit
import SwiftUI
import Testing

@testable import PostioKit

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
        let shown = PillsRow.fitting(widths: [200, 200, 200], spacing: 8, room: 420)
        #expect(shown == 2)
    }

    @Test func pillsThatFitAreAllShown() {
        #expect(PillsRow.fitting(widths: [100, 100], spacing: 8, room: 500) == 2)
    }
}
