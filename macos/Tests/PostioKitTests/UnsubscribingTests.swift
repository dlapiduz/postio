import Testing
@testable import PostioKit

@Suite struct UnsubscribingTests {
    @Test func aMessageNobodyHasActedOnIsOffered() {
        let unsubscribing = Unsubscribing()
        #expect(unsubscribing.state(of: 7) == .offered)
    }

    @Test func startingSaysSoAndOnlyForThatMessage() {
        var unsubscribing = Unsubscribing()
        let began = unsubscribing.begin(7)
        #expect(began)
        #expect(unsubscribing.state(of: 7) == .leaving)
        #expect(unsubscribing.state(of: 8) == .offered)
    }

    @Test func aSecondPressWhileTheFirstIsInFlightDoesNotRecordTwice() {
        // `X` held down, or the key and the button in quick succession: one activation, because the
        // log is the user's record of what they left.
        var unsubscribing = Unsubscribing()
        let first = unsubscribing.begin(7)
        #expect(first)
        let second = unsubscribing.begin(7)
        #expect(!second)
    }

    @Test func aRecordedActivationTakesTheBannerAway() {
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
        var unsubscribing = Unsubscribing()
        _ = unsubscribing.begin(7)
        unsubscribing.withdraw(7)
        #expect(unsubscribing.state(of: 7) == .offered)
    }

    @Test func aLeftListThatLeftMailOffersToClearItAndAnAnswerSettlesIt() {
        var unsubscribing = Unsubscribing()
        let sweep = Unsubscribing.Sweep(
            summary: "Unsubscribed from news.example.org. 14 messages from it are still here",
            button: "Archive 14", delete: false)
        unsubscribing.offerSweep(7, sweep)
        #expect(unsubscribing.state(of: 7) == .offered, "not left, so nothing to clear")

        _ = unsubscribing.begin(7)
        unsubscribing.finish(7, complaint: nil)
        unsubscribing.offerSweep(7, sweep)
        #expect(unsubscribing.state(of: 7) == .clearing(sweep))
        let again = unsubscribing.begin(7)
        #expect(!again, "pressing Unsubscribe again does not restart it")

        unsubscribing.answerSweep(7)
        #expect(unsubscribing.state(of: 7) == .left)
    }
}
