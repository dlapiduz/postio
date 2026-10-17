import Testing

@testable import PostioKit

/// The composer's body follows the window the person sized, as the message
/// window's column does (spec 009 M1, extended to the composer): a fixed
/// 720 left a band of nothing beside the text in a wide window.
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
