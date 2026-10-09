import Testing

@testable import PostioAppKit

struct MotionTests {
    @Test func reduceMotionRemovesTheTravelRatherThanShorteningIt() {
        // Asked for by people for whom movement is a symptom. A 50ms slide is
        // still a slide.
        #expect(Motion.duration(reduceMotion: true) == 0)
        #expect(Motion.duration(reduceMotion: false) <= 0.1, "PRODUCT.md §18's budget")
        #expect(Motion.duration(reduceMotion: false) > 0)
    }
}
