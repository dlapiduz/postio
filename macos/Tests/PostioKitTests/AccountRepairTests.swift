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
        // A password field here would ask for something no provider would
        // accept.
        let said = offer(account(repair: .browser))
        #expect(said?.button == "Reconnect")
        #expect(said?.reason == "This account's sign-in has expired.")
    }

    @Test func anExpiredAppPasswordAsksForOneRatherThanOpeningABrowser() {
        // "Reconnect" in front of a password field is a promise about a
        // browser that is not going to open, so the two routes do not share
        // a word.
        #expect(offer(account(repair: .password))?.button == "New password…")
    }

    @Test func aRotatedPasswordTheServerRefusedIsOfferedARepair() {
        // The common case, and the one the row used to have no button for:
        // nothing expires, the server simply says no. Only a test finds it,
        // and what it found has to turn into something to press (#1584).
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
        // A row with nothing in the Keychain used to get a sentence and no
        // way forward. It gets the same password route a rotated one does,
        // without removing the account and adding it again.
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
        // A broker-fed account: Postio holds no client and no credential to
        // put right. The row says so; a dead button would be worse than none.
        let said = offer(account(needsAttention: false, repair: .nothing), found: .rejected)
        #expect(said?.button == nil)
        #expect(said?.reason == "The server refused this account's credential.")
    }

    @Test func anExpiryTheBoundaryReportedOutranksAnOlderFinding() {
        // The boundary's flag is read on every settings open; a test's finding
        // is from whenever somebody pressed the button.
        let said = offer(account(repair: .browser), found: .missing)
        #expect(said?.reason == "This account's sign-in has expired.")
    }

    @Test func theButtonSaysWhatItIsDoingWhileItWorks() {
        // A browser sign-in returns when somebody comes back from a tab,
        // which can be a minute. A button that still says "Reconnect" reads
        // as one that did not take the click.
        #expect(offer(account(repair: .browser), running: true)?.button == "Signing in…")
        #expect(offer(account(repair: .password), running: true)?.button == "Saving…")
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
