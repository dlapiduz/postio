import Testing

@testable import PostioKit

/// The reader's zoom: fixed steps, one reader-wide preference (spec 006
/// FR-021, FR-021b, FR-021d; #1705).
@Suite struct ReaderZoomTests {
    /// The design's steps, as `[reader] zoom` takes them.
    private let steps: [UInt16] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300]

    @Test func zoomingInTakesTheNextStepNotAFixedAmount() {
        var zoom = ReaderZoom(percent: 100, steps: steps)
        let moved1 = zoom.zoomIn()
        #expect(moved1)
        #expect(zoom.percent == 110)
        let moved2 = zoom.zoomIn()
        #expect(moved2)
        #expect(zoom.percent == 125)
    }

    @Test func zoomingOutTakesThePreviousStep() {
        var zoom = ReaderZoom(percent: 100, steps: steps)
        let moved3 = zoom.zoomOut()
        #expect(moved3)
        #expect(zoom.percent == 90)
    }

    @Test func theEndsAreTheEnds() {
        // Pressing past the last step changes nothing, and says so -- the
        // caller writes the file only when something moved.
        var big = ReaderZoom(percent: 300, steps: steps)
        let moved4 = big.zoomIn()
        #expect(!moved4)
        #expect(big.percent == 300)
        var small = ReaderZoom(percent: 50, steps: steps)
        let moved5 = small.zoomOut()
        #expect(!moved5)
        #expect(small.percent == 50)
    }

    @Test func resetIsOneHundredPercent() {
        var zoom = ReaderZoom(percent: 175, steps: steps)
        let moved6 = zoom.reset()
        #expect(moved6)
        #expect(zoom.percent == 100)
        let moved7 = zoom.reset()
        #expect(!moved7, "already there")
    }

    @Test func aValueBetweenStepsStepsFromWhereItIs() {
        // A hand-edited `zoom = 105` is honoured as it is, and the next press
        // moves to the step beyond it rather than snapping back first.
        var zoom = ReaderZoom(percent: 105, steps: steps)
        let moved8 = zoom.zoomIn()
        #expect(moved8)
        #expect(zoom.percent == 110)
        var down = ReaderZoom(percent: 105, steps: steps)
        let moved9 = down.zoomOut()
        #expect(moved9)
        #expect(down.percent == 100)
    }

    @Test func theWebViewIsGivenAFactor() {
        #expect(ReaderZoom(percent: 125, steps: steps).factor == 1.25)
    }
}
