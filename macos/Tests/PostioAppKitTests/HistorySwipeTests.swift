import CoreGraphics
import Testing

@testable import PostioAppKit
@testable import PostioKit

struct HistorySwipeTests {
    @Test func aSwipeRightIsBackAndLeftIsForward() {
        #expect(HistorySwipe.command(forAmount: 1) == "history_back")
        #expect(HistorySwipe.command(forAmount: -1) == "history_forward")
        #expect(HistorySwipe.command(forAmount: 0.2) == nil, "a swipe that did not go far is nothing")
        #expect(HistorySwipe.command(forSwipeDeltaX: -1) == "history_back")
        #expect(HistorySwipe.command(forSwipeDeltaX: 1) == "history_forward")
    }
}
