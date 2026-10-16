import PostioFFI
import Testing

@testable import PostioKit

/// The three account actions, and the one that has to be asked about (#1277).
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
        // It takes the account's mail with it. A destructive action that
        // happens on the first click is one people undo by restoring a
        // backup.
        let actions = AccountActions()
        actions.askToRemove(account())

        #expect(actions.confirmingRemoval?.address == "ada@example.com")
        #expect(!actions.isBusy, "and nothing has happened yet")
    }

    @Test func theConfirmationNamesWhatGoesAndWhatDoesNot() {
        // "Are you sure?" is a dialog people learn to dismiss without
        // reading. This one says the address, that the mail goes, that the
        // password goes, and that the server is untouched.
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
        // The settings window opens whether or not a store did: settings are
        // a file, and being unable to read mail is not being unable to
        // configure it.
        let actions = AccountActions()
        await actions.test(account(), through: nil)

        #expect(!actions.isBusy)
        #expect(actions.outcome == nil, "nothing was attempted, so nothing is reported")
    }

    @Test func nothingIsPartialUntilSomethingHasBeenTested() {
        // The state is a *finding*, not a guess: asking the keyring on every
        // settings open would raise a permission prompt per account.
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
        // A rotated app password: the one a retry never fixes, so it has to
        // become an offer rather than another "Test connection" (#1584).
        let actions = AccountActions()
        actions.found(report(rejected: true), for: 1)
        #expect(actions.trouble(for: 1) == .rejected)
    }

    @Test func aServerThatDidNotAnswerIsNotACredentialProblem() {
        // The control: an unreachable host is not cured by a new password,
        // and offering one sends somebody to replace what was never wrong.
        let actions = AccountActions()
        actions.found(report(), for: 1)
        #expect(actions.trouble(for: 1) == nil)
    }

    @Test func aFindingIsAboutTheAccountThatWasTested() {
        // One model serves every row. A finding that followed the selection
        // would tell the next account it has no password.
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

    // -- what a long re-index says while it runs (#1284) -------------------

    @Test func aReindexThatIsNotRunningIgnoresAReportThatArrivesLate() {
        // Events arrive whenever they arrive. One landing after the pass has
        // finished must not make an idle pane look busy.
        let actions = AccountActions()
        actions.reindexProgressed(done: 10, total: 100)

        #expect(actions.progress == nil)
        #expect(actions.progressLabel == nil)
    }

    @Test func theLabelReadsTheWayAPersonCounts() {
        // Five-digit numbers are read wrong without separators, and this is
        // the number that says whether to wait or go and make tea.
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

/// Telling the application an account arrived (#1299).
///
/// The sheet writes the row through the boundary and that is all it can do:
/// the engines were started when the session opened, so a new account has
/// none, syncs nothing, and reads to the user as one that did not save. It
/// did save — a relaunch made it appear and sync fifteen folders, which is
/// exactly the tell.
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
        // The settings window can be built without an application behind it —
        // settings are a file, and being unable to read mail is not being
        // unable to configure it.
        let actions = AccountActions()
        actions.added()
    }
}
