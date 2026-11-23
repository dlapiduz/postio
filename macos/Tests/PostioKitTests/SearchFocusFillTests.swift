import SwiftUI
import Testing

@testable import PostioKit

struct SearchFocusFillTests {
    @Test func theFocusedTintIsTheDesignsInEitherAppearance() {
        #expect(SearchRuns.focusFill(.light) == 0.08)
        #expect(SearchRuns.focusFill(.dark) == 0.14)
    }
}
