import Testing

@testable import PostioKit

/// Where `space` has paged the reader to, and when the page should scroll.
@Suite struct ReaderPagesTests {
    @Test func aNewMessageStartsAtTheTopWithoutScrollingTheOneOnScreen() {
        // A move sets the count back to the top so the next `space` pages
        // from there. It used to bump the token as well, and a new token is
        // the web view's cue to scroll: it scrolled the *outgoing* page, a
        // beat before the new one replaced it, and the message jumped on
        // every move. A freshly loaded page starts at the top on its own.
        var pages = ReaderPages()
        pages.turn(forward: true)
        let asked = pages.token

        pages.newMessage()

        #expect(pages.page == 0)
        #expect(pages.token == asked, "nothing is asked to scroll")
    }

    @Test func aTurnAsksForAScrollEvenWhenTheNumberDoesNotMove() {
        // Paging up at the top leaves the number at 0; the token is what
        // says a turn was asked for at all.
        var pages = ReaderPages()
        pages.turn(forward: false)
        pages.turn(forward: false)
        #expect(pages.page == 0)
        #expect(pages.token == 2)
    }
}
