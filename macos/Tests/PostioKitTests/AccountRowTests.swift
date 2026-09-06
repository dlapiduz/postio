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
        // Canvas 3d: never a shrug. It used to point at `postio-provision`,
        // because there was no way in from the interface; now there is one,
        // and the sentence names it (#1279).
        let empty = AccountRow.emptyMessage
        #expect(empty.contains("+"), "\(empty)")
        #expect(!empty.contains("postio-provision"), "the terminal is no longer the way in")
    }

    @Test func anExpiredTokenIsSomethingToDoSomethingAbout() {
        // The canvas draws a warning and an inline Reconnect for exactly this
        // state: the account is there, the mail is there, and nothing will
        // arrive until somebody signs in again.
        #expect(AccountRow.needsAttention(account(facts: ["outlook", "oauth2", "token expired"])))
        #expect(!AccountRow.needsAttention(account(facts: ["imap", "password", "4291 msg"])))
    }
}
