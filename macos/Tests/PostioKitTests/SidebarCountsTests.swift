import PostioFFI
import Testing

@testable import PostioKit

/// Which events move a folder's unread and total counts.
///
/// The sidebar's badges were refreshed on `mailboxesChanged` and on nothing
/// else — and the engine emits that only when the folder *set* moves, which
/// is folder discovery. So: read a message and Inbox's badge did not
/// decrement, archive one and nothing moved, new mail arrived and the count
/// was unchanged. Until you switched folders, which incidentally re-emitted
/// the event, and everything jumped at once.
///
/// GTK states the same rule in `crates/postio-gtk/src/feed.rs`, with the same
/// reasoning: *"Counts move with read state and with mail arriving or
/// leaving. Which mailbox is irrelevant — the sidebar shows all of them."*
@Suite struct SidebarCountsTests {
    @Test func readStateAndMailArrivingOrLeavingMoveTheCounts() {
        #expect(SidebarCounts.movedBy(.messagesChanged(account: 1, messages: [7])))
        #expect(SidebarCounts.movedBy(.messagesRemoved(account: 1, mailbox: 2, messages: [7])))
        #expect(SidebarCounts.movedBy(.newMail(account: 1, mailbox: 2, messages: [7])))
        #expect(SidebarCounts.movedBy(.messageListChanged(account: 1, mailbox: 2)))
    }

    @Test func theFolderSetMovingStillCountsBecauseTheRowsThemselvesChange() {
        #expect(SidebarCounts.movedBy(.mailboxesChanged(account: 1)))
    }

    @Test func nothingElseCostsAReadOfEveryFolder() {
        // A folder read is a store round trip per account, so the events
        // that cannot have moved a count must not ask for one. `cursorMoved`
        // in particular fires on every `j`.
        #expect(!SidebarCounts.movedBy(.cursorMoved(row: 3, message: 7)))
        #expect(!SidebarCounts.movedBy(.pageReady(page: 0)))
        #expect(!SidebarCounts.movedBy(.conversationReady(thread: 9)))
        #expect(!SidebarCounts.movedBy(.reindexProgress(account: 1, done: 1, total: 2)))
        #expect(!SidebarCounts.movedBy(.syncProgress(account: 1, done: 1, total: 2)))
        #expect(!SidebarCounts.movedBy(.other(kind: "BackfillProgress")))
        #expect(
            !SidebarCounts.movedBy(.notice(kind: .completed, message: "Archived", undoable: true)),
            "a notice is a sentence about something that already emitted its own event"
        )
    }
}
