import PostioFFI
import Testing

@testable import PostioKit

/// When the conversation page on screen has to be composed again.
///
/// Measured, not guessed: every sync pass of every folder said "messages
/// changed", the engine bumped the page's revision for all of them, and each
/// bump composed the open page again and cancelled the compose a move was
/// waiting on.
@Suite struct PageRefreshTests {
    private let onScreen: Set<Int64> = [10, 11, 12]

    @Test func aChangeToAMessageOnThePageRefreshesIt() {
        // A body arrived, a flag moved: the page may come out different.
        #expect(PageRefresh.needed(by: .messagesChanged(account: 1, messages: [99, 11]), showing: onScreen))
    }

    @Test func aChangeElsewhereLeavesThePageAlone() {
        // The Archive folder's sync touching its own mail is nothing to do
        // with the conversation being read.
        #expect(!PageRefresh.needed(by: .messagesChanged(account: 1, messages: [40, 41]), showing: onScreen))
        #expect(!PageRefresh.needed(by: .messagesRemoved(account: 1, mailbox: 2, messages: [40]), showing: onScreen))
    }

    @Test func aMessageOnThePageGoingAwayRefreshesIt() {
        #expect(PageRefresh.needed(by: .messagesRemoved(account: 1, mailbox: 2, messages: [12]), showing: onScreen))
    }

    @Test func newMailInAFolderMayBeAReplyToThisConversation() {
        // A list change names no messages, and a reply arriving is one: the
        // page is asked again, and loaded only if it came out different.
        #expect(PageRefresh.needed(by: .messageListChanged(account: 1, mailbox: 1), showing: onScreen))
    }

    @Test func nothingIsRefreshedWhenNoConversationIsOpen() {
        #expect(!PageRefresh.needed(by: .messagesChanged(account: 1, messages: [11]), showing: []))
        #expect(!PageRefresh.needed(by: .messageListChanged(account: 1, mailbox: 1), showing: []))
    }
}
