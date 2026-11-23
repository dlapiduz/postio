import Testing

@testable import PostioKit

@Suite struct BodyHeightTests {
    @Test func aMeasurementIsUsedAsItArrives() {
        #expect(BodyHeight.clamped(420) == 420)
    }

    @Test func aBodyThatMeasuredNothingStillLeavesRoomToSaySo() {
        #expect(BodyHeight.clamped(0) == BodyHeight.minimum)
        #expect(BodyHeight.clamped(-40) == BodyHeight.minimum)
    }

    @Test func aMeasurementThatIsNotANumberDoesNotBecomeAFrame() {
        // `NaN` in a layout constraint is a crash.
        #expect(BodyHeight.clamped(.nan) == BodyHeight.minimum)
        #expect(BodyHeight.clamped(.infinity) == BodyHeight.maximum)
    }

    @Test func aRunawayDocumentIsCappedRatherThanTrusted() {
        // A sender can write a document of any height; honouring it would scroll past a single
        // message for a minute.
        #expect(BodyHeight.clamped(9_000_000) == BodyHeight.maximum)
    }
}
