import PostioFFI
import Testing
@testable import PostioKit

@MainActor
@Suite struct SavedSearchesTests {
    private func row(_ key: String, _ name: String) -> SavedSearchFfi {
        SavedSearchFfi(key: key, name: name, query: "from:ada@example.com")
    }

    private func edit(_ rows: [SavedSearchFfi], changed: String?) -> SavedSearchEditFfi {
        SavedSearchEditFfi(searches: rows, changed: changed)
    }

    @Test func nothingIsFocusedWhileTheKeyboardIsOnAFolder() {
        // `d` in the sidebar means "delete the saved search I am on", and
        // over a folder there is no such thing. It must not reach for one.
        let searches = SavedSearches()
        searches.apply(edit([row("unread", "Unread")], changed: nil))
        #expect(searches.focused == nil)
    }

    @Test func aRowThatLeftTheFileIsNotSomeOtherRow() {
        // config.toml is hand-edited and watched: a row can go while this
        // window is open. Acting on the nearest one would edit a search
        // nobody aimed at.
        let searches = SavedSearches()
        searches.apply(edit([row("unread", "Unread")], changed: nil))
        searches.put(cursor: "flagged")
        #expect(searches.focused == nil)
    }

    @Test func theCursorFollowsTheRowThatMoved() {
        // What makes a reorder repeatable. Left on the vacated position, a
        // second ⇧↑ would walk whichever row had slid into it.
        let searches = SavedSearches()
        searches.apply(edit([row("a", "A"), row("b", "B")], changed: nil))
        searches.put(cursor: "b")
        searches.apply(edit([row("b", "B"), row("a", "A")], changed: "b"))
        #expect(searches.cursor == "b")
        #expect(searches.focused?.name == "B")
    }

    @Test func aMoveThatChangedNothingLeavesTheCursorAlone() {
        // A row already at the end it was moving toward. `changed` is nil,
        // and flashing a change that did not happen is worse than nothing.
        let searches = SavedSearches()
        searches.apply(edit([row("a", "A"), row("b", "B")], changed: nil))
        searches.put(cursor: "a")
        searches.apply(edit([row("a", "A"), row("b", "B")], changed: nil))
        #expect(searches.cursor == "a")
    }

    @Test func aRenameWishCarriesTheNameToStartFrom() {
        // The field opens with the current name in it: renaming is usually
        // editing, and an empty box asks somebody to retype what is there.
        let searches = SavedSearches()
        searches.ask(.rename(key: "unread", from: "Unread"))
        #expect(searches.wish == .rename(key: "unread", from: "Unread"))
    }

    @Test func twoOfTheSameWishInARowAreTwoWishes() {
        let searches = SavedSearches()
        searches.ask(.confirmDelete(key: "a", name: "A"))
        let first = searches.wishToken
        searches.ask(.confirmDelete(key: "a", name: "A"))
        #expect(searches.wishToken > first)
    }
}
