import Testing
@testable import PostioKit

@Suite struct SearchContextTests {
    @Test func aListOfResultsIsTheSearchContext() {
        // Where `o` and `⌘⇧S` are pressed: over the results, having left the
        // field. Reading the context off the field's focus put them in the
        // one place a bare letter cannot resolve.
        #expect(SearchContext.list(showingResults: true) == .search)
    }

    @Test func aListOfAMailboxIsJustTheList() {
        // A mailbox has no other order to offer and no query to keep, so
        // neither command belongs here — and `o` must stay free for whatever
        // the list wants it for.
        #expect(SearchContext.list(showingResults: false) == .list)
    }
}
