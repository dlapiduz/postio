import PostioFFI
import Testing
@testable import PostioKit

@MainActor
@Suite struct SettingsAccountsTests {
    private func account(_ id: Int64, _ address: String) -> AccountFfi {
        AccountFfi(
            id: id,
            address: address,
            displayName: address,
            initials: "AL",
            isDefault: false,
            enabled: true,
            facts: [],
            needsAttention: false,
            repair: .nothing
        )
    }

    @Test func aimingAtNothingHitsNothing() {
        // The whole of ADR 0005 Q6c's reasoning: `d` with no row under the
        // keyboard must remove nothing. Falling back to the first account
        // would take somebody's mail off this Mac on a keystroke aimed at
        // no row at all.
        let pane = SettingsAccounts()
        #expect(pane.focused(in: [account(1, "ada@example.com")]) == nil)
    }

    @Test func theCursorNamesTheRowTheKeyboardIsOn() {
        let pane = SettingsAccounts()
        pane.put(cursor: 2)
        let row = pane.focused(in: [account(1, "ada@example.com"), account(2, "grace@example.test")])
        #expect(row?.address == "grace@example.test")
    }

    @Test func aRowRemovedUnderTheCursorIsNotSomeOtherRow() {
        // Two settings windows, or a removal that landed while the command
        // was in flight. The cursor still holds an id and the id is gone;
        // the answer is nothing, never "the nearest one".
        let pane = SettingsAccounts()
        pane.put(cursor: 2)
        #expect(pane.focused(in: [account(1, "ada@example.com")]) == nil)
    }

    @Test func twoOfTheSameWishInARowAreTwoWishes() {
        // Add an account, cancel the sheet, press it again. A view watching
        // the value alone would see nothing the second time.
        let pane = SettingsAccounts()
        pane.ask(.add)
        let first = pane.wishToken
        pane.ask(.add)
        #expect(pane.wish == .add)
        #expect(pane.wishToken > first)
    }

    @Test func aCredentialWishNamesTheAccountItIsFor() {
        // The sheet asks for one account's password. Carrying the id with
        // the wish is what stops it being asked for whichever row the pane
        // happens to be drawing when the sheet opens.
        let pane = SettingsAccounts()
        pane.ask(.updateCredential(7))
        #expect(pane.wish == .updateCredential(7))
    }
}
