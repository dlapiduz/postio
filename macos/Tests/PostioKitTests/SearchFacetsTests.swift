import PostioFFI
import Testing

@testable import PostioKit

/// The scope rail's rows and the refine chips, for the results on screen
/// (#1157).
///
/// Both come from one boundary answer -- `searchFacets` -- so one model holds
/// them and both surfaces read it. Two views measuring separately paid for the
/// same second pass over the index twice per keystroke.
@MainActor
@Suite struct SearchFacetsTests {
    private func count(_ scope: SearchScopeFfi, _ label: String, _ hits: UInt64) -> ScopeCountFfi {
        ScopeCountFfi(scope: scope, label: label, hits: hits, spoken: "\(label), \(hits) matches")
    }

    private func measured() -> SearchFacetsFfi {
        SearchFacetsFfi(
            scopes: [
                count(.allMail, "All mail", 14),
                count(.inbox, "Inbox only", 3),
                count(.lists, "Lists", 0),
            ],
            refinements: [RefinementFfi(token: "is:unread", hits: 4)]
        )
    }

    @Test func theRailIsEveryScopeInTheBoundarysOrderWithTheCurrentOneMarked() {
        let facets = SearchFacets()
        facets.take(measured())

        let rows = facets.rows(current: .inbox)

        #expect(rows.map(\.label) == ["All mail", "Inbox only", "Lists"])
        #expect(rows.map(\.hits) == [14, 3, 0], "a zero is drawn: an empty scope is worth knowing")
        #expect(rows.map(\.selected) == [false, true, false])
        #expect(rows[1].spoken == "Inbox only, 3 matches")
    }

    @Test func theChipsComeFromTheSameAnswer() {
        let facets = SearchFacets()
        facets.take(measured())
        #expect(facets.refinements.map(\.token) == ["is:unread"])
    }

    @Test func aNewResultSetKeepsTheLastCountsUntilItsOwnLand() {
        // Measured off the main actor, so there is a moment between a query
        // running and its counts arriving. Rows that vanished for it would
        // make the rail jump under the pointer on every keystroke.
        let facets = SearchFacets()
        facets.take(measured())
        facets.resultsChanged(searching: true)
        #expect(facets.rows(current: .allMail).count == 3)
    }

    @Test func leavingSearchEmptiesTheRailAndTheChips() {
        let facets = SearchFacets()
        facets.take(measured())
        facets.resultsChanged(searching: false)
        #expect(facets.rows(current: .allMail).isEmpty)
        #expect(facets.refinements.isEmpty)
    }

    @Test func theSidebarsKeyboardWalksTheRailWhileItIsShowing() {
        // While the rail stands where the folders were, `j` and `k` walk it:
        // the folders are not on screen, and stepping onto one would leave
        // the search for a folder nobody could see being chosen. Clamped
        // rather than wrapped, like the folder walk.
        let facets = SearchFacets()
        facets.take(measured())
        #expect(facets.step(from: .allMail, by: 1) == .inbox)
        #expect(facets.step(from: .inbox, by: 1) == .lists)
        #expect(facets.step(from: .lists, by: 1) == .lists, "stops at the end")
        #expect(facets.step(from: .allMail, by: -1) == .allMail, "and at the start")
    }

    @Test func aRailWithNothingMeasuredHasNowhereToStep() {
        #expect(SearchFacets().step(from: .allMail, by: 1) == nil)
    }
}
