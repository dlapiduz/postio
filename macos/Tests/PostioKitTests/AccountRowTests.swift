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

    @Test func theRowSaysHowMuchMailTheAccountHas() {
        // Canvas 27's line: `imap · password · 4291 msg`. Summed from the
        // folder counts the window already holds — asking the store to count
        // again on every settings open would be a scan for a line nobody is
        // waiting on.
        let folders = [
            mailbox(account: 1, total: 4_000),
            mailbox(account: 1, total: 291),
            mailbox(account: 2, total: 9_820),
        ]

        let line = AccountRow.line(account(facts: ["imap", "password"]), mailboxes: folders)

        #expect(line == "imap · password · 4,291 msg", "\(line)")
    }

    @Test func theRowSaysWhatTheMailWeighsWhenSomethingKnows() {
        // Canvas 27's full line. The wording is the shared crate's, so both
        // frontends describe a store the same way.
        let line = AccountRow.line(
            account(facts: ["imap", "password"]),
            mailboxes: [mailbox(account: 1, total: 4_291)],
            weight: "1.8 GB downloaded"
        )

        #expect(line == "imap · password · 4,291 msg · 1.8 GB downloaded", "\(line)")
    }

    @Test func anAccountWithNothingToWeighSaysNothingAboutIt() {
        // `0 B` beside a freshly added account reads as a failure.
        let line = AccountRow.line(account(facts: ["imap"]), mailboxes: [], weight: nil)
        #expect(line == "imap")
    }

    @Test func anAccountWithNoMailSaysNothingAboutIt() {
        // A "0 msg" beside a freshly added account is a fact nobody needed
        // and reads as a failure.
        let line = AccountRow.line(account(facts: ["imap"]), mailboxes: [])
        #expect(line == "imap")
    }

    private func mailbox(account: Int64, total: UInt32) -> MailboxFfi {
        MailboxFfi(
            id: Int64(total),
            account: account,
            parent: nil,
            name: "Inbox",
            role: .inbox,
            unread: 0,
            total: total,
            selectable: true,
            lastSyncedAt: nil,
            special: true
        )
    }
}
