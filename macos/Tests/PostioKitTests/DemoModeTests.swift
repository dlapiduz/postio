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

    @Test func keysAreReplayedOnlyInADemo() {
        // A picture of `!` with three rows marked needs the keys pressed;
        // a real store never replays anything.
        let keys = ["POSTIO_DEMO_KEYS": "! x j x"]
        #expect(DemoMode.keys(in: keys).isEmpty)
        #expect(DemoMode.keys(in: keys.merging(["POSTIO_DEMO": "small"]) { $1 }).map(\.character)
            == ["!", "x", "j", "x"])
        #expect(DemoMode.keys(in: ["POSTIO_DEMO": "small"]).isEmpty)
    }

    @Test func aReplayedKeyIsTheKeyAsTyped() {
        let keys = DemoMode.keys(in: ["POSTIO_DEMO": "small", "POSTIO_DEMO_KEYS": "J x !"])
        #expect(keys.map(\.modifiers.shift) == [true, false, true])
        #expect(keys.allSatisfy { !$0.modifiers.command && !$0.modifiers.control && $0.name == nil })
    }
}
