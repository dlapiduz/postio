import PostioFFI
import Testing

@testable import PostioKit

/// The accounts pane's rows.
///
/// The failure this guards is the one this project keeps finding: a store with
/// accounts in it and a pane that draws none. `provisioned_account.rs` proves
/// the boundary can see them; this proves the row says what it saw.
@MainActor
@Suite struct AccountRowTests {
    private func account(
        address: String = "ada@example.com",
        isDefault: Bool = false,
        facts: [String] = ["IMAP · password"]
    ) -> AccountFfi {
        AccountFfi(
            id: 1,
            address: address,
            displayName: "Ada Lovelace",
            initials: "AL",
            isDefault: isDefault,
            facts: facts
        )
    }

    @Test func theFactsReadAsOneLineInTheOrderTheyWereGiven() {
        // Joined here, decided on the other side of the boundary: two panes
        // assembling their own facts are two descriptions of one account.
        #expect(
            AccountRow.line(account(facts: ["IMAP · password", "disabled"]))
                == "IMAP · password · disabled"
        )
    }

    @Test func onlyTheDefaultAccountWearsTheTag() {
        // Words, never colour alone — ADR 0005. And it says what the marker
        // does rather than asserting a status (#960).
        #expect(AccountRow.tag(account(isDefault: true)) == "default")
        #expect(AccountRow.tag(account(isDefault: false)) == nil)
    }

    @Test func anEmptyListSaysWhyRatherThanDrawingNothing() {
        // Canvas 3d: never a shrug. On a Mac this is also the common case —
        // there is no way to add an account from the interface yet, so the
        // sentence has to name the way in that does exist.
        let empty = AccountRow.emptyMessage
        #expect(empty.contains("postio-provision"), "\(empty)")
    }
}
