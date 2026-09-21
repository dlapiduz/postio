import PostioFFI
import SwiftUI

/// The "Saved searches" section of the sidebar.
///
/// Buttons rather than selectable rows, and deliberately: a saved search is
/// not a mailbox, so it has no `SidebarRowId`, and giving it a `.tag()` of
/// some other type is how a `List` ends up with rows it can never select.
/// Picking one hands its query to the same `Session.search` the query field
/// calls — a saved search is a query that was written down, not a second kind
/// of thing to open.
public struct SavedSearchRows: View {
    private let searches: SavedSearches
    private let run: (SavedSearchFfi) -> Void

    public init(searches: SavedSearches, run: @escaping (SavedSearchFfi) -> Void) {
        self.searches = searches
        self.run = run
    }

    public var body: some View {
        if !searches.rows.isEmpty {
            Section("Saved searches") {
                ForEach(searches.rows, id: \.key) { search in
                    Button {
                        searches.put(cursor: search.key)
                        run(search)
                    } label: {
                        HStack(spacing: PostioTokens.space2) {
                            Image(systemName: "line.3.horizontal.decrease.circle")
                                .foregroundStyle(.secondary)
                            Text(search.name)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    // The keyboard's row is drawn as such: `r`, `d` and
                    // ⇧↑/⇧↓ all act on it, and a verb whose target is
                    // invisible is a verb you press and hope about.
                    .listRowBackground(
                        Rectangle()
                            .fill(.selection)
                            .opacity(searches.cursor == search.key ? 1 : 0)
                    )
                    .accessibilityLabel("Saved search: \(search.name)")
                    .accessibilityHint(search.query)
                }
            }
            .selectionDisabled()
        }
    }
}
