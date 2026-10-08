import CoreGraphics
import Testing

@testable import PostioKit

struct DemoModeTests {
    @Test func noDemoUnlessOneIsNamed() {
        #expect(DemoMode.seed(in: [:]) == nil)
        #expect(DemoMode.seed(in: ["POSTIO_DEMO": ""]) == nil)
        #expect(DemoMode.seed(in: ["POSTIO_DEMO": "small"]) == "small")
    }

    @Test func theWindowIsTheDesignsSizeUnlessToldOtherwise() {
        #expect(DemoMode.windowSize(in: [:]) == CGSize(width: 1440, height: 900))
        #expect(
            DemoMode.windowSize(in: ["POSTIO_WINDOW_SIZE": "1024x768"])
                == CGSize(width: 1024, height: 768))
        #expect(
            DemoMode.windowSize(in: ["POSTIO_WINDOW_SIZE": "wide"])
                == CGSize(width: 1440, height: 900))
    }

    @Test func theAppearanceIsLightDarkOrTheSystems() {
        #expect(DemoMode.appearance(in: ["POSTIO_APPEARANCE": "dark"]) == "dark")
        #expect(DemoMode.appearance(in: ["POSTIO_APPEARANCE": "light"]) == "light")
        #expect(DemoMode.appearance(in: ["POSTIO_APPEARANCE": "sepia"]) == nil)
        #expect(DemoMode.appearance(in: [:]) == nil)
    }
}
