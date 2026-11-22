import Testing

@testable import PostioKit

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
        var choice = ReaderViewChoice()
        choice.toggle(7)
        choice.showOriginal(7)
        #expect(!choice.isReduced(7))
        choice.showOriginal(7)
        #expect(!choice.isReduced(7))
    }

    @Test func leavingTheConversationForgetsEverything() {
        var choice = ReaderViewChoice()
        choice.toggle(7)
        choice.toggle(8)
        choice.clear()
        #expect(choice.messages.isEmpty)
    }
}
