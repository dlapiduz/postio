import PostioFFI
import Testing

@testable import PostioKit

@MainActor
@Suite struct AccountActionsTests {
    private func account(_ address: String = "ada@example.com") -> AccountFfi {
        AccountFfi(
            id: 1,
            address: address,
            displayName: "Ada Lovelace",
            initials: "AL",
            isDefault: false,
            enabled: true,
            facts: ["imap", "password"],
            needsAttention: false,
            repair: .nothing
        )
    }

    @Test func nothingIsRunningAndNothingHasBeenSaid() {
        let actions = AccountActions()
        #expect(!actions.isBusy)
        #expect(actions.outcome == nil)
    }

    @Test func removingIsAskedAboutRatherThanDone() {
        let actions = AccountActions()
        actions.askToRemove(account())

        #expect(actions.confirmingRemoval?.address == "ada@example.com")
        #expect(!actions.isBusy, "and nothing has happened yet")
    }

    @Test func theConfirmationNamesWhatGoesAndWhatDoesNot() {
        let actions = AccountActions()
        let warning = actions.removalWarning(for: account())

        #expect(warning.contains("ada@example.com"))
        #expect(warning.contains("Keychain"))
        #expect(warning.lowercased().contains("server is not touched"))
    }

    @Test func changingYourMindLeavesTheAccountAlone() {
        let actions = AccountActions()
        actions.askToRemove(account())
        actions.cancelRemoval()

        #expect(actions.confirmingRemoval == nil)
    }

    @Test func anActionWithNoSessionSaysAndDoesNothing() async {
        let actions = AccountActions()
        await actions.test(account(), through: nil)

        #expect(!actions.isBusy)
        #expect(actions.outcome == nil, "nothing was attempted, so nothing is reported")
    }

    @Test func nothingIsPartialUntilSomethingHasBeenTested() {
        let actions = AccountActions()
        #expect(actions.trouble(for: 1) == nil)
    }

    private func report(
        reachable: Bool = false,
        missing: Bool = false,
        rejected: Bool = false
    ) -> ConnectionReportFfi {
        ConnectionReportFfi(
            reachable: reachable,
            missingCredential: missing,
            rejectedCredential: rejected,
            message: "what the test said"
        )
    }

    @Test func aTestThatFoundNoCredentialIsThePartialState() {
        let actions = AccountActions()
        actions.found(report(missing: true), for: 1)
        #expect(actions.trouble(for: 1) == .missing)
    }

    @Test func aTestTheServerRefusedIsARejectedCredential() {
        let actions = AccountActions()
        actions.found(report(rejected: true), for: 1)
        #expect(actions.trouble(for: 1) == .rejected)
    }

    @Test func aServerThatDidNotAnswerIsNotACredentialProblem() {
        let actions = AccountActions()
        actions.found(report(), for: 1)
        #expect(actions.trouble(for: 1) == nil)
    }

    @Test func aFindingIsAboutTheAccountThatWasTested() {
        let actions = AccountActions()
        actions.found(report(rejected: true), for: 1)
        #expect(actions.trouble(for: 2) == nil)
    }

    @Test func aRepairedCredentialStopsBeingReportedAsBroken() {
        let actions = AccountActions()
        actions.found(report(missing: true), for: 1)
        actions.credentialRepaired(1)
        #expect(actions.trouble(for: 1) == nil)
    }

    @Test func renamingWithNoSessionChangesNothingAndSaysNothing() {
        let actions = AccountActions()
        actions.rename(account(), to: "Ada at work", through: nil)

        #expect(actions.outcome == nil)
    }

    @Test func aReindexThatIsNotRunningIgnoresAReportThatArrivesLate() {
        let actions = AccountActions()
        actions.reindexProgressed(done: 10, total: 100)

        #expect(actions.progress == nil)
        #expect(actions.progressLabel == nil)
    }

    @Test func theLabelReadsTheWayAPersonCounts() {
        let actions = AccountActions()
        actions.beginReindexForTesting()
        actions.reindexProgressed(done: 1_203, total: 4_985)

        #expect(actions.progressLabel == "1,203 of 4,985")
    }

    @Test func aTotalOfZeroDrawsNothingRatherThanZeroOfZero() {
        let actions = AccountActions()
        actions.beginReindexForTesting()
        actions.reindexProgressed(done: 0, total: 0)

        #expect(actions.progressLabel == nil)
    }
}

@MainActor
@Suite struct AccountAddedTests {
    @Test func addingAnAccountTellsWhoeverIsListening() {
        let actions = AccountActions()
        var told = 0
        actions.accountAdded = { told += 1 }

        actions.added()

        #expect(told == 1)
    }

    @Test func nobodyListeningIsHarmless() {
        let actions = AccountActions()
        actions.added()
    }
}
