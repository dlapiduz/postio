import PostioFFI
import Testing

@testable import PostioKit

@MainActor
@Suite struct AccountRowTests {
    private func account(
        address: String = "ada@example.com",
        isDefault: Bool = false,
        facts: [String] = ["IMAP · password"],
        needsAttention: Bool = false,
        repair: RepairRouteFfi = .nothing
    ) -> AccountFfi {
        AccountFfi(
            id: 1,
            address: address,
            displayName: "Ada Lovelace",
            initials: "AL",
            isDefault: isDefault,
            enabled: true,
            facts: facts,
            needsAttention: needsAttention,
            repair: repair
        )
    }

    @Test func theFactsReadAsOneLineInTheOrderTheyWereGiven() {
        #expect(
            AccountRow.line(account(facts: ["IMAP · password", "disabled"]))
                == "IMAP · password · disabled"
        )
    }

    @Test func onlyTheDefaultAccountWearsTheTag() {
        #expect(AccountRow.tag(account(isDefault: true)) == "default")
        #expect(AccountRow.tag(account(isDefault: false)) == nil)
    }

    @Test func anEmptyListSaysWhyRatherThanDrawingNothing() {
        let empty = AccountRow.emptyMessage
        #expect(empty.contains("+"), "\(empty)")
        #expect(!empty.contains("postio-provision"), "the terminal is no longer the way in")
    }

    @Test func anExpiredTokenIsSomethingToDoSomethingAbout() {
        #expect(
            AccountRow.needsAttention(
                account(facts: ["outlook · oauth2"], needsAttention: true, repair: .browser)
            )
        )
        #expect(!AccountRow.needsAttention(account(facts: ["imap · password · 4291 msg"])))
    }

    @Test func theRowSaysHowMuchMailTheAccountHas() {
        let folders = [
            mailbox(account: 1, total: 4_000),
            mailbox(account: 1, total: 291),
            mailbox(account: 2, total: 9_820),
        ]

        let line = AccountRow.line(account(facts: ["imap", "password"]), mailboxes: folders)

        #expect(line == "imap · password · 4,291 msg", "\(line)")
    }

    @Test func theRowSaysWhatTheMailWeighsWhenSomethingKnows() {
        let line = AccountRow.line(
            account(facts: ["imap", "password"]),
            mailboxes: [mailbox(account: 1, total: 4_291)],
            weight: "1.8 GB downloaded"
        )

        #expect(line == "imap · password · 4,291 msg · 1.8 GB downloaded", "\(line)")
    }

    @Test func anAccountWithNothingToWeighSaysNothingAboutIt() {
        let line = AccountRow.line(account(facts: ["imap"]), mailboxes: [], weight: nil)
        #expect(line == "imap")
    }

    @Test func theViewsAreNotCountedAsMoreMail() {
        let flaggedView = MailboxFfi(
            id: 0, account: 1, parent: nil, name: "Flagged", role: .flagged,
            unread: 0, total: 9, selectable: true, lastSyncedAt: nil,
            special: true, flagged: 9, snoozed: 0
        )
        let line = AccountRow.line(
            account(facts: ["imap"]),
            mailboxes: [mailbox(account: 1, total: 63), flaggedView]
        )
        #expect(line == "imap · 63 msg")
    }

    @Test func anAccountWithNoMailSaysNothingAboutIt() {
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
            special: true,
            flagged: 0,
            snoozed: 0
        )
    }

    @Test func attentionComesFromTheBoundaryRatherThanFromAWordInTheFactLine() {
        #expect(!AccountRow.needsAttention(account(facts: ["outlook · oauth2"])))
        #expect(
            AccountRow.needsAttention(
                account(facts: ["outlook · oauth2"], needsAttention: true, repair: .browser)
            ),
            "the boundary says it needs a person and the row disagrees"
        )
    }

    @Test func aFactLineMentioningExpiryDoesNotSpeakForTheBoundary() {
        #expect(!AccountRow.needsAttention(account(facts: ["token expired"])))
    }

}
