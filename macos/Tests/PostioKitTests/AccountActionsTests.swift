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
}
