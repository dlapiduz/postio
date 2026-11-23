import Testing

@testable import PostioKit

@Suite struct SessionLifetimeTests {
    @Test func puttingTheWindowAwayDoesNotStopTheMail() {
        #expect(!SessionLifetime.shouldEnd(on: .background))
        #expect(!SessionLifetime.shouldEnd(on: .inactive))
        #expect(!SessionLifetime.shouldEnd(on: .active))
    }

    @Test func quittingEndsIt() {
        #expect(SessionLifetime.shouldEnd(on: .terminating))
    }

    @Test func everyPhaseHasAnAnswer() {
        for phase in SessionPhase.allCases {
            _ = SessionLifetime.shouldEnd(on: phase)
        }
    }
}
