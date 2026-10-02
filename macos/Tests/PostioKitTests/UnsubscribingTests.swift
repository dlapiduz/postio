import Testing
@testable import PostioKit

/// The unsubscribe banner's state, held where a command can reach it (#1706).
@Suite struct UnsubscribingTests {
    @Test func aMessageNobodyHasActedOnIsOffered() {
        let unsubscribing = Unsubscribing()
        #expect(unsubscribing.state(of: 7) == .offered)
    }

    @Test func startingSaysSoAndOnlyForThatMessage() {
        // A conversation can hold eight messages from four lists; leaving one
        // says nothing about the others.
        var unsubscribing = Unsubscribing()
        let began = unsubscribing.begin(7)
        #expect(began)
        #expect(unsubscribing.state(of: 7) == .leaving)
        #expect(unsubscribing.state(of: 8) == .offered)
    }

    @Test func aSecondPressWhileTheFirstIsInFlightDoesNotRecordTwice() {
        // `X` held down, or the key and the button in quick succession: one
        // activation, because the log is the user's record of what they left.
        var unsubscribing = Unsubscribing()
        let first = unsubscribing.begin(7)
        #expect(first)
        let second = unsubscribing.begin(7)
        #expect(!second)
    }

    @Test func aRecordedActivationTakesTheBannerAway() {
        // The banner disappearing *is* the success; there is no tick.
        var unsubscribing = Unsubscribing()
        _ = unsubscribing.begin(7)
        unsubscribing.finish(7, complaint: nil)
        #expect(unsubscribing.state(of: 7) == .left)
        let again = unsubscribing.begin(7)
        #expect(!again, "a list already left is not left again")
    }

    @Test func aRefusalIsSaidAndCanBeTriedAgain() {
        var unsubscribing = Unsubscribing()
        _ = unsubscribing.begin(7)
        unsubscribing.finish(7, complaint: "The store would not take a write.")
        #expect(unsubscribing.state(of: 7) == .failed("The store would not take a write."))
        let retried = unsubscribing.begin(7)
        #expect(retried, "a failure is not a reason to refuse the next press")
    }

    @Test func aPressOnAMessageWithNothingToLeaveLeavesNoTrace() {
        // GTK's rule for the key: where there is no banner, nothing happens.
        var unsubscribing = Unsubscribing()
        _ = unsubscribing.begin(7)
        unsubscribing.withdraw(7)
        #expect(unsubscribing.state(of: 7) == .offered)
    }
}
