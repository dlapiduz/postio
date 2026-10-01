import PostioFFI
import SwiftUI

/// The commands the search box offers while it holds `>` -- the panel under
/// the field, which keeps the keyboard.
///
/// Every row is `session.paletteEntries`, already ranked and filtered to what
/// can run here by `postio_ui::palette`, the matcher GTK's box uses: the same
/// query offers the same commands on both platforms. Nothing here sorts.
public struct CommandResults: View {
    /// One row's height -- its text and its padding -- and the tallest the
    /// panel grows before it scrolls.
    static let rowHeight: CGFloat = 30
    static let maxHeight: CGFloat = 360

    private let rows: [PaletteEntryFfi]
    private let highlighted: Int
    private let run: (String) -> Void

    public init(rows: [PaletteEntryFfi], highlighted: Int, run: @escaping (String) -> Void) {
        self.rows = rows
        self.highlighted = highlighted
        self.run = run
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if rows.isEmpty {
                // A panel that draws nothing looks broken; this one says the
                // box was understood and nothing matched.
                Text("No command matches")
                    .foregroundStyle(.secondary)
                    .padding(PostioTokens.space4)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                ScrollViewReader { scroller in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(rows.enumerated()), id: \.element.id) { index, entry in
                                row(entry, isHighlighted: index == highlighted)
                                    .id(index)
                                    .contentShape(.rect)
                                    .onTapGesture { run(entry.id) }
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
        .accessibilityLabel("Commands")
    }

    private func row(_ entry: PaletteEntryFfi, isHighlighted: Bool) -> some View {
        HStack(spacing: PostioTokens.space3) {
            // The matched characters, emphasised from the offsets the shared
            // matcher returned -- the numbers GTK turns into Pango bold.
            Text(PaletteRow.highlighted(entry))
                .lineLimit(1)
            Spacer(minLength: PostioTokens.space3)
            KeyCaps(binding: entry.binding)
        }
        .padding(.horizontal, PostioTokens.space4)
        .frame(height: Self.rowHeight)
        .background(isHighlighted ? Color.accentColor.opacity(0.22) : .clear)
    }
}
