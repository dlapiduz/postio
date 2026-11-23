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
        // `d` with no row under the keyboard must remove nothing: falling back to the first account
        // would take someone's mail on a keystroke aimed at no row (ADR 0005 Q6c).
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
        // The cursor still holds an id that is gone; the answer is nothing, never "the nearest
        // one".
        let pane = SettingsAccounts()
        pane.put(cursor: 2)
        #expect(pane.focused(in: [account(1, "ada@example.com")]) == nil)
    }

    @Test func twoOfTheSameWishInARowAreTwoWishes() {
        // A view watching the value alone would see nothing the second time.
        let pane = SettingsAccounts()
        pane.ask(.add)
        let first = pane.wishToken
        pane.ask(.add)
        #expect(pane.wish == .add)
        #expect(pane.wishToken > first)
    }

    @Test func aCredentialWishNamesTheAccountItIsFor() {
        let pane = SettingsAccounts()
        pane.ask(.updateCredential(7))
        #expect(pane.wish == .updateCredential(7))
    }

    @Test func aWishIsTakenOnceByWhicheverPaneIsThereToTakeIt() {
        // The pane that appears did not see the token move, so it takes the wish on appearing;
        // taken, not read, so a second appearance opens no second sheet.
        let accounts = SettingsAccounts()
        accounts.ask(.add)
        #expect(accounts.take() == .add)
        #expect(accounts.take() == nil)
    }
}
