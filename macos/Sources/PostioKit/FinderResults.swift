import PostioFFI
import SwiftUI

/// What the search box offers while it asks one of its modes' questions --
/// the panel under the field, which keeps the keyboard (`FinderBox`).
///
/// Every row comes from the boundary already ranked by the shared matcher --
/// `paletteEntries` for `>`, `finderFolders`/`Contacts`/`Labels` for the
/// rest -- so the same text offers the same rows on both platforms. Nothing
/// here sorts.
public struct FinderResults: View {
    /// One row's height -- its text and its padding -- and the tallest the
    /// panel grows before it scrolls.
    static let rowHeight: CGFloat = 30
    static let maxHeight: CGFloat = 360

    private let rows: [FinderRow]
    private let empty: String
    private let highlighted: Int
    private let pick: (Int) -> Void

    public init(rows: [FinderRow], empty: String, highlighted: Int, pick: @escaping (Int) -> Void) {
        self.rows = rows
        self.empty = empty
        self.highlighted = highlighted
        self.pick = pick
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if rows.isEmpty {
                // Never a shrug: the boundary's sentence names what was
                // looked in.
                Text(empty)
                    .foregroundStyle(.secondary)
                    .padding(PostioTokens.space4)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                ScrollViewReader { scroller in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(rows.enumerated()), id: \.offset) { index, entry in
                                row(entry, isHighlighted: index == highlighted)
                                    .id(index)
                                    .contentShape(.rect)
                                    .onTapGesture { pick(index) }
                            }
                        }
                    }
                    .onChange(of: highlighted) { _, now in scroller.scrollTo(now) }
                }
                // As tall as its rows, up to a cap: a scroll view takes all
                // the height it is offered, and a panel of five commands
                // drew five rows over a box of empty space.
                .frame(height: min(CGFloat(rows.count) * Self.rowHeight, Self.maxHeight))
            }
        }
        .frame(width: 460)
        .background(.regularMaterial, in: .rect(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.primary.opacity(0.12)))
        .shadow(color: .black.opacity(0.25), radius: 12, y: 4)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Matches")
    }

    private func row(_ entry: FinderRow, isHighlighted: Bool) -> some View {
        HStack(spacing: PostioTokens.space3) {
            // The matched characters, emphasised from the offsets the shared
            // matcher returned -- the numbers GTK turns into Pango bold.
            Text(PaletteRow.highlighted(title: entry.title, positions: entry.positions))
                .lineLimit(1)
            Spacer(minLength: PostioTokens.space3)
            if let detail = entry.detail {
                Text(detail)
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            // Only a command has a key; a folder or a person has nothing to
            // press, and a dash on every row would say otherwise.
            if entry.binding != nil {
                KeyCaps(binding: entry.binding)
            }
        }
        .padding(.horizontal, PostioTokens.space4)
        .frame(height: Self.rowHeight)
        .background(isHighlighted ? Color.accentColor.opacity(0.22) : .clear)
    }
}
