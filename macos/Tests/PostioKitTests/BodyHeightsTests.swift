import Testing
@testable import PostioKit

@MainActor
@Suite struct BodyHeightsTests {
    @Test func aFirstOpeningHasNothingRemembered() {
        // Which is the pop: the minimum height, then the measurement. The
        // cache's whole job is to make that a first-time-only cost.
        let heights = BodyHeights()
        #expect(heights.height(for: 7) == nil)
    }

    @Test func aRevisitGetsLastTimesMeasurement() {
        let heights = BodyHeights()
        heights.remember(431, for: 7)
        #expect(heights.height(for: 7) == 431)
    }

    @Test func aRemeasurementReplacesTheOldNumber() {
        // The pane got wider, the document laid out shorter. The
        // measurement always runs; the cache must follow it, not argue.
        let heights = BodyHeights()
        heights.remember(431, for: 7)
        heights.remember(215, for: 7)
        #expect(heights.height(for: 7) == 215)
    }

    @Test func theCacheDropsWholeRatherThanGrowingForever() {
        let heights = BodyHeights()
        for id in 0..<Int64(BodyHeights.capacity) {
            heights.remember(100, for: id)
        }
        heights.remember(100, for: 9_999)
        #expect(heights.height(for: 0) == nil, "the old entries went")
        #expect(heights.height(for: 9_999) == 100, "the new one stayed")
    }
}
