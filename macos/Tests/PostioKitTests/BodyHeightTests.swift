import Testing

@testable import PostioKit

/// How tall an expanded message's body is drawn (#1263).
///
/// A conversation stacks its bodies, so each web view has to be exactly as
/// tall as what it holds — a scroll view inside a scroll view is the shape
/// every mail client that gets this wrong has. The height is measured from
/// the rendered document, and a measurement is a number from outside: it
/// arrives late, zero, absurd, or not at all.
@Suite struct BodyHeightTests {
    @Test func aMeasurementIsUsedAsItArrives() {
        #expect(BodyHeight.clamped(420) == 420)
    }

    @Test func aBodyThatMeasuredNothingStillLeavesRoomToSaySo() {
        // Zero happens: the document is loaded but not laid out yet. A row of
        // no height reads as a message with nothing in it.
        #expect(BodyHeight.clamped(0) == BodyHeight.minimum)
        #expect(BodyHeight.clamped(-40) == BodyHeight.minimum)
    }

    @Test func aMeasurementThatIsNotANumberDoesNotBecomeAFrame() {
        // `NaN` in a layout constraint is a crash, not a bad height.
        #expect(BodyHeight.clamped(.nan) == BodyHeight.minimum)
        #expect(BodyHeight.clamped(.infinity) == BodyHeight.maximum)
    }

    @Test func aRunawayDocumentIsCappedRatherThanTrusted() {
        // A sender can write a document of any height, and a stack that
        // honoured one would scroll for a minute past a single message.
        #expect(BodyHeight.clamped(9_000_000) == BodyHeight.maximum)
    }
}
