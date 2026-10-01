import Testing

@testable import PostioKit

/// Find in the open message or conversation (spec 006 FR-018, #1705).
@Suite struct FindInMessageTests {
    @Test func theBarIsClosedUntilAskedFor() {
        let find = FindInMessage()
        #expect(!find.isOpen)
        #expect(find.request == nil, "nothing is searched for before anybody types")
    }

    @Test func openingAsksForTheFieldEveryTime() {
        // `⌘F` with the bar already open puts the keyboard back in the field,
        // which is what a second press means everywhere on a Mac.
        var find = FindInMessage()
        find.open()
        let first = find.focusToken
        find.open()
        #expect(find.isOpen)
        #expect(find.focusToken != first)
    }

    @Test func typingSearchesForward() {
        var find = FindInMessage()
        find.open()
        find.setQuery("invoice")
        #expect(find.request?.query == "invoice")
        #expect(find.request?.backwards == false)
    }

    @Test func nextAndPreviousEachAskAgainAndSayWhichWay() {
        // The web view acts on a request it has not seen, so two presses of
        // `⌘G` must be two requests even though nothing else changed.
        var find = FindInMessage()
        find.open()
        find.setQuery("invoice")
        let typed = find.request
        let next = find.next()
        #expect(next)
        #expect(find.request != typed)
        #expect(find.request?.backwards == false)
        let previous = find.previous()
        #expect(previous)
        #expect(find.request?.backwards == true)
    }

    @Test func stepsWithNothingToFindOpenTheBarInstead() {
        // `⌘G` before anything was searched for is a person who wants to
        // search; answering "nothing to find" would be a dead key.
        var find = FindInMessage()
        let stepped = find.next()
        #expect(!stepped)
        #expect(find.isOpen)
    }

    @Test func closingForgetsTheHighlightButKeepsTheWords() {
        // `⌘G` after Escape finds the same word again, as in every Mac app.
        var find = FindInMessage()
        find.open()
        find.setQuery("invoice")
        find.close()
        #expect(!find.isOpen)
        #expect(find.request == nil)
        #expect(find.query == "invoice")
        let again = find.next()
        #expect(again)
        #expect(find.request?.query == "invoice")
    }

    @Test func whatTheViewFoundIsReportedBack() {
        var find = FindInMessage()
        find.open()
        find.setQuery("nowhere")
        find.found(false)
        #expect(find.missing)
        find.setQuery("invoice")
        #expect(!find.missing, "a new search has not failed yet")
    }
}
