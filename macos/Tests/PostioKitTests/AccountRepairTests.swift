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

    private func offer(
        _ account: AccountFfi,
        found: CredentialTrouble? = nil,
        running: Bool = false
    ) -> AccountRepair.Offer? {
        AccountRepair.offer(for: account, found: found, running: running)
    }

    @Test func ahealthyAccountHasNothingToPressAndNothingToSay() {
        #expect(offer(account(needsAttention: false)) == nil)
    }

    @Test func anExpiredGrantIsReconnectedInABrowser() {
        let said = offer(account(repair: .browser))
        #expect(said?.button == "Reconnect")
        #expect(said?.reason == "This account's sign-in has expired.")
    }

    @Test func anExpiredAppPasswordAsksForOneRatherThanOpeningABrowser() {
        #expect(offer(account(repair: .password))?.button == "New password…")
    }

    @Test func aRotatedPasswordTheServerRefusedIsOfferedARepair() {
        let said = offer(account(needsAttention: false, repair: .password), found: .rejected)
        #expect(said?.button == "New password…")
        #expect(said?.reason == "The server refused this account's password.")
    }

    @Test func aRevokedGrantTheServerRefusedIsReconnected() {
        let said = offer(account(needsAttention: false, repair: .browser), found: .rejected)
        #expect(said?.button == "Reconnect")
        #expect(said?.reason == "The server refused this account's sign-in.")
    }

    @Test func thePartialStateIsNoLongerADeadEnd() {
        let said = offer(account(needsAttention: false, repair: .password), found: .missing)
        #expect(said?.button == "New password…")
        #expect(said?.reason == "This account has no password in your Keychain yet.")
    }

    @Test func aPartialOAuthAccountSignsInAgainRatherThanAskingForAPassword() {
        let said = offer(account(needsAttention: false, repair: .browser), found: .missing)
        #expect(said?.button == "Reconnect")
        #expect(said?.reason == "This account has no sign-in in your Keychain yet.")
    }

    @Test func anAccountWithNothingToSignIntoSaysWhatIsWrongWithNoButton() {
        let said = offer(account(needsAttention: false, repair: .nothing), found: .rejected)
        #expect(said?.button == nil)
        #expect(said?.reason == "The server refused this account's credential.")
    }

    @Test func anExpiryTheBoundaryReportedOutranksAnOlderFinding() {
        let said = offer(account(repair: .browser), found: .missing)
        #expect(said?.reason == "This account's sign-in has expired.")
    }

    @Test func theButtonSaysWhatItIsDoingWhileItWorks() {
        #expect(offer(account(repair: .browser), running: true)?.button == "Signing in…")
        #expect(offer(account(repair: .password), running: true)?.button == "Saving…")
    }

    @Test func askingForAPasswordStartsFromAnEmptyField() {
        let repair = AccountRepair()
        repair.typed = "left over"
        repair.ask(account(repair: .password))
        #expect(repair.asking == 1)
        #expect(repair.typed.isEmpty)
    }

    @Test func cancellingForgetsWhatWasTyped() {
        let repair = AccountRepair()
        repair.ask(account(repair: .password))
        repair.typed = "an app password"
        repair.cancel()
        #expect(repair.asking == nil)
        #expect(repair.typed.isEmpty)
    }
}
