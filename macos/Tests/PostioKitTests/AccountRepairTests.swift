import PostioFFI
import Testing
@testable import PostioKit

@MainActor
@Suite struct AccountRepairTests {
    private func account(
        needsAttention: Bool = true,
        repair: RepairRouteFfi = .browser
    ) -> AccountFfi {
        AccountFfi(
            id: 1,
            address: "ada@example.com",
            displayName: "Ada Lovelace",
            initials: "AL",
            isDefault: false,
            enabled: true,
            facts: [],
            needsAttention: needsAttention,
            repair: repair
        )
    }

    @Test func ahealthyAccountHasNothingToPress() {
        #expect(AccountRepair.label(for: account(needsAttention: false), running: false) == nil)
    }

    @Test func anExpiredGrantIsReconnectedInABrowser() {
        // A password field here would ask for something no provider would
        // accept.
        #expect(AccountRepair.label(for: account(repair: .browser), running: false) == "Reconnect")
    }

    @Test func arotatedAppPasswordAsksForOneRatherThanOpeningABrowser() {
        // "Reconnect" in front of a password field is a promise about a
        // browser that is not going to open, so the two routes do not share
        // a word.
        #expect(
            AccountRepair.label(for: account(repair: .password), running: false) == "New password…"
        )
    }

    @Test func anAccountWithNothingToSignIntoHasNoButton() {
        // A local mail store. The row still warns; there is simply nothing
        // to press, and a dead button is worse than none.
        #expect(AccountRepair.label(for: account(repair: .nothing), running: false) == nil)
    }

    @Test func theButtonSaysWhatItIsDoingWhileItWorks() {
        // A browser sign-in returns when somebody comes back from a tab,
        // which can be a minute. A button that still says "Reconnect" reads
        // as one that did not take the click.
        #expect(AccountRepair.label(for: account(repair: .browser), running: true) == "Signing in…")
        #expect(AccountRepair.label(for: account(repair: .password), running: true) == "Saving…")
    }

    @Test func askingForAPasswordStartsFromAnEmptyField() {
        // Never pre-filled, and never with the old one: the old one is what
        // stopped working, and Postio does not have it to offer anyway.
        let repair = AccountRepair()
        repair.typed = "left over"
        repair.ask(account(repair: .password))
        #expect(repair.asking == 1)
        #expect(repair.typed.isEmpty)
    }

    @Test func cancellingForgetsWhatWasTyped() {
        // A password that stayed in memory behind a closed field is a
        // password in memory for no reason.
        let repair = AccountRepair()
        repair.ask(account(repair: .password))
        repair.typed = "an app password"
        repair.cancel()
        #expect(repair.asking == nil)
        #expect(repair.typed.isEmpty)
    }
}
