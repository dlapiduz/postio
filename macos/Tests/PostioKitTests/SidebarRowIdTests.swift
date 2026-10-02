import PostioFFI
import Testing

@testable import PostioKit

/// What makes two sidebar rows the same row.
///
/// The list identified rows by `mailbox.id`, which is fine for folders and
/// wrong for the three that are not folders. Flagged, Snoozed and the Outbox
/// are queries, built by `postio_ui::sidebar::view_rows` with no id at all —
/// so every one of them is id `0`, and so is every one of them on the second
/// account.
///
/// SwiftUI's `ForEach(id:)` and `List(selection:)` both take that at its
/// word. Two rows claiming one identity is undefined behaviour in a list:
/// selection lands on whichever the diffing algorithm decided was "the" row,
/// and it is not stable between redraws.
@Suite struct SidebarRowIdTests {
    private func row(_ role: MailboxRoleFfi, id: Int64 = 0, account: Int64 = 7) -> MailboxFfi {
        MailboxFfi(
            id: id, account: account, parent: nil, name: "", role: role,
            unread: 0, total: 0, selectable: true, lastSyncedAt: nil,
            special: true, flagged: 0, snoozed: 0
        )
    }

    @Test func twoViewRowsInOneAccountAreTwoRows() {
        #expect(row(.flagged).rowId != row(.snoozed).rowId)
        #expect(row(.snoozed).rowId != row(.outbox).rowId)
    }

    @Test func theSameViewInTwoAccountsIsTwoRows() {
        #expect(row(.flagged, account: 1).rowId != row(.flagged, account: 2).rowId)
    }

    @Test func aFolderIsStillIdentifiedByItsFolder() {
        #expect(row(.inbox, id: 42).rowId == row(.inbox, id: 42).rowId)
        #expect(row(.inbox, id: 42).rowId != row(.inbox, id: 43).rowId)
    }

    @Test func everyRowTheSidebarDrawsHasItsOwnIdentity() {
        // The shape that broke: one account's special section, with both
        // view rows in it. Every identity distinct, or the list is
        // undefined.
        let drawn = [
            row(.inbox, id: 1), row(.sent, id: 2), row(.drafts, id: 3),
            row(.archive, id: 4), row(.trash, id: 5), row(.junk, id: 6),
            row(.flagged), row(.snoozed), row(.outbox),
        ]
        #expect(Set(drawn.map(\.rowId)).count == drawn.count)
    }
}
