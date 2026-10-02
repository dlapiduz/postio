import Testing

@testable import PostioKit

/// Reader view, one message at a time (spec 006 FR-031, #1705).
///
/// Every message opens as its sender built it; reader view is a command away
/// (`⇧⌘O`), and *View original* (`⌘O`) is the way back. Both are commands, so
/// the choice lives where a command can reach it rather than in a view.
@Suite struct ReaderViewChoiceTests {
    @Test func aMessageOpensAsItsSenderBuiltIt() {
        let choice = ReaderViewChoice()
        #expect(!choice.isReduced(7))
        #expect(choice.messages.isEmpty)
    }

    @Test func readerViewIsAboutOneMessage() {
        var choice = ReaderViewChoice()
        choice.toggle(7)
        #expect(choice.isReduced(7))
        #expect(!choice.isReduced(8), "a conversation is several messages and this is about one")
        #expect(choice.messages == [7])
    }

    @Test func togglingTwiceComesBack() {
        var choice = ReaderViewChoice()
        choice.toggle(7)
        choice.toggle(7)
        #expect(!choice.isReduced(7))
    }

    @Test func viewOriginalLeavesReaderViewAndOnlyLeavesIt() {
        // `⌘O` is not a toggle: pressed over a message already shown as sent
        // it does nothing, rather than reducing it.
        var choice = ReaderViewChoice()
        choice.toggle(7)
        choice.showOriginal(7)
        #expect(!choice.isReduced(7))
        choice.showOriginal(7)
        #expect(!choice.isReduced(7))
    }

    @Test func leavingTheConversationForgetsEverything() {
        // "Per view": the next conversation opens as sent whatever was done
        // to the last one.
        var choice = ReaderViewChoice()
        choice.toggle(7)
        choice.toggle(8)
        choice.clear()
        #expect(choice.messages.isEmpty)
    }
}
