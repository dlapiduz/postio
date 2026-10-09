import CoreGraphics
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// The trackpad's back and forward are the history's commands
/// (specs/010-focus-search T073): the swipe is reported as `history_back`
/// or `history_forward`, and the controller decides whether there is
/// anywhere to go.
struct HistorySwipeTests {
    @Test func aSwipeRightIsBackAndLeftIsForward() {
        #expect(HistorySwipe.command(forAmount: 1) == "history_back")
        #expect(HistorySwipe.command(forAmount: -1) == "history_forward")
        #expect(HistorySwipe.command(forAmount: 0.2) == nil, "a swipe that did not go far is nothing")
        #expect(HistorySwipe.command(forSwipeDeltaX: -1) == "history_back")
        #expect(HistorySwipe.command(forSwipeDeltaX: 1) == "history_forward")
    }
}
