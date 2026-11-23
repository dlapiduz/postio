import Testing

@testable import PostioKit

@Suite struct FindInMessageTests {
    @Test func theBarIsClosedUntilAskedFor() {
        let find = FindInMessage()
        #expect(!find.isOpen)
        #expect(find.request == nil, "nothing is searched for before anybody types")
    }

    @Test func openingAsksForTheFieldEveryTime() {
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
        // Two presses of `⌘G` must be two requests though nothing else changed: the web view acts
        // on a request it has not seen.
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
        var find = FindInMessage()
        let stepped = find.next()
        #expect(!stepped)
        #expect(find.isOpen)
    }

    @Test func closingForgetsTheHighlightButKeepsTheWords() {
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
