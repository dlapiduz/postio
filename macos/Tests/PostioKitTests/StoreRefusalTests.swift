import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// What a start over answers in these tests.
private let startedOver = StartedOverFfi(setAside: "/stores/set-aside/when", accounts: 2)

/// The page a store that will not open shows on launch, and "Start a fresh
/// store" (specs/009-focus-macos T100). The words are GTK's, read across the
/// boundary (`store_refusal_words`); the remedy is the boundary's too -- a
/// store from another build is `StoreFromAnotherBuild`, which trying again
/// can never get past, and everything else is worth opening again.
@MainActor
@Suite struct StoreRefusalTests {
    let words = storeRefusalWords()

    /// A start over that can be held part way, and counts its calls.
    final class Gate: @unchecked Sendable {
        let held = DispatchSemaphore(value: 0)
        private let lock = NSLock()
        private var calls = 0
        var count: Int { lock.withLock { calls } }
        func call() { lock.withLock { calls += 1 } }
    }

    // MARK: what the page says

    @Test func aStoreFromAnotherBuildOffersAFreshStoreInGTKsWords() {
        let refusal = StoreRefusal(
            SessionError.StoreFromAnotherBuild(message: "The store is at version 99."), words: words)
        #expect(refusal.remedy == .startOver)
        #expect(refusal.heading == words.fromAnotherBuild)
        #expect(refusal.sentence == words.startOverSentence)
        #expect(refusal.button == words.startOver)
    }

    @Test func aStoreThatMayOpenNextTimeOffersToTryAgainAndSaysWhy() {
        let refusal = StoreRefusal(
            SessionError.StoreUnavailable(message: "Another Postio has this store open."), words: words)
        #expect(refusal.remedy == .tryAgain)
        #expect(refusal.heading == words.cantOpen)
        #expect(refusal.sentence == "Another Postio has this store open.")
        #expect(refusal.button == words.tryAgain)
    }

    @Test func aLockedKeychainIsTriedAgainNotStartedOver() {
        // ADR 0014: a locked keyring is "unlock this and retry", never a
        // reason to set somebody's mail aside.
        let refusal = StoreRefusal(
            SessionError.KeyringLocked(message: "Unlock the login keychain."), words: words)
        #expect(refusal.remedy == .tryAgain)
        #expect(refusal.sentence == "Unlock the login keychain.")
    }

    @Test func aSentenceIsTheBoundarysNotADebugDescription() {
        let refusal = StoreRefusal(
            SessionError.RuntimeUnavailable(message: "No threads."), words: words)
        #expect(refusal.sentence == "No threads.")
        #expect(!refusal.sentence.contains("RuntimeUnavailable"))
    }

    // MARK: what the button does

    @Test func startingOverSetsTheStoreAsideThenOpensTheFreshOne() async {
        var reopened: [StartedOverFfi?] = []
        let model = StoreRefusalModel(
            refusal: StoreRefusal(SessionError.StoreFromAnotherBuild(message: ""), words: words),
            words: words,
            startOver: { startedOver },
            reopen: { reopened.append($0) })
        await model.act()
        #expect(reopened == [startedOver], "opened once, told where the old store went")
    }

    @Test func whileItRunsTheButtonSaysSoAndASecondPressDoesNothing() async {
        let gate = Gate()
        var reopened = 0
        let model = StoreRefusalModel(
            refusal: StoreRefusal(SessionError.StoreFromAnotherBuild(message: ""), words: words),
            words: words,
            startOver: {
                gate.call()
                gate.held.wait()
                return startedOver
            },
            reopen: { _ in reopened += 1 })
        let first = Task { await model.act() }
        // Bounded: a model that never starts must fail, not hang.
        for _ in 0..<10_000 where !model.working { await Task.yield() }
        #expect(model.working)
        #expect(model.button == words.startingOver)
        await model.act()
        gate.held.signal()
        await first.value
        #expect(gate.count == 1, "one start over, however often it was pressed")
        #expect(reopened == 1)
        #expect(!model.working)
    }

    @Test func aStartOverThatFailedSaysWhyAndOffersToTryAgain() async {
        let gate = Gate()
        var reopened: [StartedOverFfi?] = []
        let model = StoreRefusalModel(
            refusal: StoreRefusal(SessionError.StoreFromAnotherBuild(message: ""), words: words),
            words: words,
            startOver: {
                gate.call()
                throw SessionError.StoreUnavailable(message: "Another Postio has this store open.")
            },
            reopen: { reopened.append($0) })
        await model.act()
        #expect(reopened.isEmpty, "nothing to open")
        #expect(model.refusal.heading == words.cantOpen)
        #expect(model.refusal.sentence == "Another Postio has this store open.")
        #expect(model.button == words.tryAgain)
        // Trying again opens the store again -- which, if it is still the
        // old one, offers the start over again, as GTK's page does.
        await model.act()
        #expect(reopened == [nil])
        #expect(gate.count == 1, "the failed start over is not run again by Try again")
    }

    @Test func tryingAgainOpensAgainWithoutStartingOver() async {
        let gate = Gate()
        var reopened: [StartedOverFfi?] = []
        let model = StoreRefusalModel(
            refusal: StoreRefusal(SessionError.KeyringLocked(message: "Locked."), words: words),
            words: words,
            startOver: {
                gate.call()
                return startedOver
            },
            reopen: { reopened.append($0) })
        await model.act()
        #expect(reopened == [nil])
        #expect(gate.count == 0, "a locked keychain never sets the store aside")
    }
}
