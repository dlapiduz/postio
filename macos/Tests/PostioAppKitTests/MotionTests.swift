import Testing

@testable import PostioAppKit

struct MotionTests {
    @Test func reduceMotionRemovesTheTravelRatherThanShorteningIt() {
        #expect(Motion.duration(reduceMotion: true) == 0)
        #expect(Motion.duration(reduceMotion: false) <= 0.1, "PRODUCT.md §18's budget")
        #expect(Motion.duration(reduceMotion: false) > 0)
    }
}
