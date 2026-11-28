import Testing

@testable import PostioKit

@Suite struct ReaderPagesTests {
    @Test func aNewMessageStartsAtTheTopWithoutScrollingTheOneOnScreen() {
        var pages = ReaderPages()
        pages.turn(forward: true)
        let asked = pages.token

        pages.newMessage()

        #expect(pages.page == 0)
        #expect(pages.token == asked, "nothing is asked to scroll")
    }

    @Test func aTurnAsksForAScrollEvenWhenTheNumberDoesNotMove() {
        var pages = ReaderPages()
        pages.turn(forward: false)
        pages.turn(forward: false)
        #expect(pages.page == 0)
        #expect(pages.token == 2)
    }
}
