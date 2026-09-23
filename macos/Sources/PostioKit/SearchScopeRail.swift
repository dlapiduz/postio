import AppKit
import PostioFFI
import SwiftUI

/// Where the search looks — All mail, Inbox only, Lists — with what each
/// would find, in place of the folders while results are showing (canvas 5,
/// #1157).
///
/// In place of the folders because that is the question the sidebar is
/// answering while the list is a result set: not "which folder" but "how much
/// of the mailbox". `Escape` puts the folders back, and the rail says so,
/// because a sidebar that changed shape with no way back named is one people
/// think they broke.
///
/// Picking a row asks the *same query* again inside that scope; the query is
/// never edited to say `in:inbox`, so what was typed stays what was typed.
public struct SearchScopeRail: View {
    private let rows: [SearchFacets.Row]
    private let pick: (SearchScopeFfi) -> Void
    /// The key that leaves search, as the keymap has it.
    private let backKey: String?

    public init(rows: [SearchFacets.Row], backKey: String?, pick: @escaping (SearchScopeFfi) -> Void) {
        self.rows = rows
        self.backKey = backKey
        self.pick = pick
    }

    public var body: some View {
        List {
            Section("Search in") {
                ForEach(rows, id: \.label) { row in
                    Button {
                        pick(row.scope)
                    } label: {
                        HStack {
                            Text(row.label)
                                .fontWeight(row.selected ? .semibold : .regular)
                            Spacer(minLength: PostioTokens.space2)
                            Text(Int(row.hits).formatted(.number))
                                .font(.system(.callout, design: .monospaced))
                                .foregroundStyle(.secondary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(
                        Rectangle()
                            .fill(.selection)
                            .opacity(row.selected ? 1 : 0)
                    )
                    .accessibilityLabel(row.spoken)
                    .accessibilityAddTraits(row.selected ? .isSelected : [])
                }
            }
            if let backKey {
                Section {
                    Text("\(backKey) returns to your folders")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .selectionDisabled()
    }
}
