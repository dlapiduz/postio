import AppKit
import Testing
@testable import PostioAppKit
@testable import PostioKit

@Suite struct AppSurfaceTests {
    private func rgb(_ c: NSColor, _ appearance: NSAppearance.Name) -> (r: Int, g: Int, b: Int) {
        var out = (r: 0, g: 0, b: 0)
        NSAppearance(named: appearance)?.performAsCurrentDrawingAppearance {
            let s = c.usingColorSpace(.sRGB) ?? c
            out = (
                Int((s.redComponent * 255).rounded()),
                Int((s.greenComponent * 255).rounded()),
                Int((s.blueComponent * 255).rounded())
            )
        }
        return out
    }

    @Test func theWindowIsOnTheSameHueFamilyAsTheReadersPaper() {
        // The reader's ground has equal red and green; an AppKit blue-grey window around it reads
        // maroon.
        let dark = rgb(AppSurface.background, .darkAqua)
        #expect(dark.r == dark.g, "the window picked up a cast: \(dark)")

        let paper = rgb(PostioTokens.colorNeutral900, .darkAqua)
        #expect(paper.r == paper.g)
        #expect((dark.b - dark.r) == (paper.b - paper.r) || abs((dark.b - dark.r) - (paper.b - paper.r)) <= 1)
    }

    @Test func theReadersPaperStaysLighterThanTheWindowInDark() {
        // The window sits just below `--color-neutral-100` so the paper reads as paper; an exact
        // match flattens the card.
        let window = rgb(AppSurface.background, .darkAqua)
        let paper = rgb(PostioTokens.colorNeutral900, .darkAqua)
        #expect(window.r < paper.r, "the window is not below the paper: \(window) vs \(paper)")
        #expect(
            paper.r - window.r == 3,
            "the gap is not the light palette's three steps: \(window) vs \(paper)"
        )
    }

    @Test func lightIsTheDesignedColourRatherThanADerivedOne() {
        let light = rgb(AppSurface.background, .aqua)
        #expect(light == (242, 242, 243))
    }

    @Test func loweringCannotIntroduceACast() {
        let neutral = NSColor(srgbRed: 0.5, green: 0.5, blue: 0.6, alpha: 1)
        let out = AppSurface.lowered(neutral, by: 0.1).usingColorSpace(.sRGB)!
        #expect(abs(out.redComponent - out.greenComponent) < 0.0001)
        #expect(
            abs((out.blueComponent - out.redComponent) - 0.1) < 0.0001,
            "the blue lift did not survive the move"
        )
    }
}
