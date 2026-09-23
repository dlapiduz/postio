import Observation
import PostioFFI

/// What the results on screen are made of: the scope rail's rows and the
/// refine chips (#1157).
///
/// One boundary answer, `searchFacets`, holds both, so one model holds them
/// and the rail and the refine bar both read it. They used to measure on
/// their own, and the second view would have paid for the same second pass
/// over the index again on every keystroke.
///
/// No AppKit (#1264): a result set's scopes are the same thing on a phone.
@MainActor
@Observable
public final class SearchFacets {
    /// Every scope with its count, in the boundary's order. Empty when no
    /// search is showing.
    public private(set) var counts: [ScopeCountFfi] = []

    /// The refine chips worth offering, best first.
    public private(set) var refinements: [RefinementFfi] = []

    public init() {}

    /// Take what the boundary measured.
    public func take(_ facets: SearchFacetsFfi) {
        counts = facets.scopes
        refinements = facets.refinements
    }

    /// The list stopped or started being a result set.
    ///
    /// A new result set keeps the last counts until its own land: they are
    /// measured off the main actor, and rows that vanished for that moment
    /// would make the rail jump under the pointer on every keystroke. Leaving
    /// search empties both, because a rail about a search nobody is in is a
    /// rail about nothing.
    public func resultsChanged(searching: Bool) {
        guard !searching else { return }
        counts = []
        refinements = []
    }

    /// One row of the rail.
    public struct Row: Equatable, Sendable {
        public let scope: SearchScopeFfi
        /// `Inbox only` — the boundary's word.
        public let label: String
        /// What switching to this scope would find, zero included.
        public let hits: UInt64
        /// What a screen reader hears: `Inbox only, 3 matches`.
        public let spoken: String
        /// Whether the search is looking here now.
        public let selected: Bool
    }

    /// The rail, with `current` marked.
    public func rows(current: SearchScopeFfi) -> [Row] {
        counts.map { count in
            Row(
                scope: count.scope,
                label: count.label,
                hits: count.hits,
                spoken: count.spoken,
                selected: count.scope == current
            )
        }
    }

    /// The scope `delta` rows from `current`, for the sidebar's keyboard
    /// while the rail is showing.
    ///
    /// Clamped rather than wrapped, `SidebarWalk`'s rule and its reason.
    /// `nil` when nothing has been measured yet, so there is no rail to walk.
    public func step(from current: SearchScopeFfi, by delta: Int) -> SearchScopeFfi? {
        guard !counts.isEmpty else { return nil }
        let order = counts.map(\.scope)
        let at = order.firstIndex(of: current) ?? 0
        return order[min(max(at + delta, 0), order.count - 1)]
    }
}
