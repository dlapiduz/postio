import Testing

@testable import PostioKit

struct ComposeColumnTests {
    @Test func aWideWindowIsReadAcrossItsWholeColumn() {
        #expect(ComposeColumn.width(window: 1440) == 960, "up to M1's measure, not 720")
        #expect(ComposeColumn.width(window: 980) == 900, "the window less 80")
    }

    @Test func aNarrowWindowGivesTheColumnItsWidthLessTheGutters() {
        #expect(ComposeColumn.width(window: 640) == 560)
    }

    @Test func beforeTheWindowHasAWidthNothingIsCapped() {
        #expect(ComposeColumn.width(window: 0) == .infinity)
    }
}
