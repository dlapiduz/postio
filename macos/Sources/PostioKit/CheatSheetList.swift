import PostioFFI
import SwiftUI

/// The keyboard sheet's body: its sections laid across columns, each command
/// beside its keys.
///
/// Grouped the way the boundary groups them -- the same
/// `postio_ui::cheatsheet::sections` the GTK overlay draws from -- and laid
/// out the way that overlay lays them: across before down, a narrow column
/// per run of sections, the key in a cap just after its name
/// (`CheatSheetLayout`). One list in a wide sheet put every key a sheet's
/// width from the name it belongs to.
///
/// In PostioKit rather than beside the sheet's window so it can be drawn
/// without one.
public struct CheatSheetList: View {
    private let sections: [CheatSectionFfi]

    /// How many columns the sheet runs to, and how wide each is. Three at
    /// 248pt is the sheet's 880pt less its gutters.
    public static let columnCount = 3
    public static let columnWidth: CGFloat = 248

    public init(sections: [CheatSectionFfi]) {
        self.sections = sections
    }

    public var body: some View {
        ScrollView {
            HStack(alignment: .top, spacing: PostioTokens.space8) {
                ForEach(
                    Array(CheatSheetLayout.columns(sections, count: Self.columnCount).enumerated()),
                    id: \.offset
                ) { _, column in
                    VStack(alignment: .leading, spacing: PostioTokens.space6) {
                        ForEach(column, id: \.title) { section in
                            SectionBlock(section: section)
                        }
                    }
                    .frame(width: Self.columnWidth, alignment: .leading)
                }
            }
            .padding(.horizontal, PostioTokens.space6)
            .padding(.vertical, PostioTokens.space4)
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
    }
}

/// One heading and its rows. A `Grid` per section, not a lazy stack across
/// the sheet: identities belong to their section by construction, which is
/// what the lazy stack got wrong when it drew whole sections blank.
private struct SectionBlock: View {
    let section: CheatSectionFfi

    var body: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space2) {
            // The canvas's kicker: small, spaced capitals, quieter than the
            // rows it heads.
            Text(section.title.uppercased())
                .font(.system(size: 11, weight: .semibold).width(.condensed))
                .tracking(1.2)
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
            Grid(alignment: .leading, horizontalSpacing: PostioTokens.space3, verticalSpacing: 5) {
                ForEach(Array(section.rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        Text(row.title)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        KeyCaps(binding: row.binding)
                            .gridColumnAlignment(.trailing)
                    }
                    .accessibilityElement(children: .ignore)
                    // A sentence, not "Archive a" -- the boundary writes it
                    // so a screen reader hears a fact rather than two columns.
                    .accessibilityLabel(row.spoken)
                }
            }
        }
    }
}
