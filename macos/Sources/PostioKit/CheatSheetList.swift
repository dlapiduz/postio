import PostioFFI
import SwiftUI

/// The keyboard sheet's rows, grouped the way the boundary groups them --
/// the same `postio_ui::cheatsheet::sections` the GTK overlay draws from, so
/// the two platforms teach the same sheet.
///
/// In PostioKit rather than beside the sheet's window so it can be drawn
/// without one, which is how its rows are checked.
public struct CheatSheetList: View {
    private let sections: [CheatSectionFfi]

    public init(sections: [CheatSectionFfi]) {
        self.sections = sections
    }

    public var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(sections, id: \.title) { section in
                    Text(section.title)
                        .font(.callout.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 16)
                        .padding(.top, 14)
                        .padding(.bottom, 4)
                        .accessibilityAddTraits(.isHeader)
                    // Identified by section *and* position. By position alone
                    // every section had a row 0, a row 1, ... and the lazy
                    // stack, which flattens nested `ForEach`es into one list,
                    // took them for the first section's rows: each later
                    // section drew nothing for as many rows as the first one
                    // had, and left the space.
                    ForEach(
                        section.rows.enumerated().map { ("\(section.title)\u{1F}\($0.offset)", $0.element) },
                        id: \.0
                    ) { _, row in
                        HStack(alignment: .firstTextBaseline) {
                            Text(row.title)
                            Spacer()
                            // The menu's glyphs, chord by chord, so `g g`
                            // still prints as its two presses.
                            Text(CheatSheetKeys.label(row.binding))
                                .font(.system(.body, design: .monospaced))
                                .foregroundStyle(row.binding == nil ? .tertiary : .secondary)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 5)
                        .accessibilityElement(children: .ignore)
                        // A sentence, not "Archive a" -- the boundary
                        // writes it so a screen reader hears a fact rather
                        // than two columns.
                        .accessibilityLabel(row.spoken)
                    }
                }
            }
        }
    }
}

