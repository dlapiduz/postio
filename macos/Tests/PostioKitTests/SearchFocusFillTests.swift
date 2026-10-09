import SwiftUI
import Testing

@testable import PostioKit

/// The tint behind search's focused row, card or way out (spec 010 T137):
/// the design's `accsoft`, the accent at 8% in light and 14% in dark, where
/// 7-9% of a dark accent was too faint to see on screen 07.
struct SearchFocusFillTests {
    @Test func theFocusedTintIsTheDesignsInEitherAppearance() {
        #expect(SearchRuns.focusFill(.light) == 0.08)
        #expect(SearchRuns.focusFill(.dark) == 0.14)
    }
}
