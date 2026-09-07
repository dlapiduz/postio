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
            facts: ["imap", "password"]
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
        #expect(!actions.missingCredential)
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
