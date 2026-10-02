import PostioFFI
import SwiftUI

/// The conversation rail (canvas 28 and 29, #1576): one row per message, the
/// one the reader is on marked, and a click to go to any of them.
///
/// Every rule is the boundary's -- which rows, how wide the window must be for
/// which presentation, and where the mark moves when a row is chosen, a key
/// is pressed or the page reports what fills the pane (`RailFfi`). This draws
/// the rows and hands the choice back.
///
/// One view for the column and for the popover a narrow window opens from
/// the header's counter: the same rows and the same click, as the brief asks
/// -- "one component with two presentations over the same data".
public struct ConversationRail: View {
    private let rows: [RailRowFfi]
    private let marked: Int?
    /// The narrower column: numbers and initials, since a truncated name is
    /// worse than none.
    private let narrow: Bool
    private let choose: (Int) -> Void

    public init(rows: [RailRowFfi], marked: Int?, narrow: Bool, choose: @escaping (Int) -> Void) {
        self.rows = rows
        self.marked = marked
        self.narrow = narrow
        self.choose = choose
    }

    /// The column's width, from the ladder the boundary names: 150 full, 118
    /// narrow.
    public static func width(narrow: Bool) -> CGFloat { narrow ? 118 : 150 }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                    Button {
                        choose(index)
                    } label: {
                        HStack(spacing: PostioTokens.space2) {
                            Text("\(row.position)")
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .frame(minWidth: 18, alignment: .trailing)
                            Text(narrow ? row.initials : row.sender)
                                .lineLimit(1)
                                .truncationMode(.tail)
                            Spacer(minLength: 0)
                            if let length = row.length, !narrow {
                                Text("\(length)")
                                    .font(.system(.caption2, design: .monospaced))
                                    .foregroundStyle(.tertiary)
                            }
                        }
                        .padding(.horizontal, PostioTokens.space2)
                        .padding(.vertical, 5)
                        .contentShape(Rectangle())
                        .background(
                            Rectangle()
                                .fill(.selection)
                                .opacity(marked == index ? 1 : 0)
                        )
                    }
                    .buttonStyle(.plain)
                    // The visible row is terse because the label carries what
                    // the eye gets from the message header instead (FR-046).
                    .accessibilityLabel(
                        "Message \(row.position) of \(rows.count), from \(row.sender), \(row.when)"
                    )
                    .accessibilityAddTraits(marked == index ? .isSelected : [])
                }
            }
            .padding(.vertical, PostioTokens.space2)
        }
        .accessibilityLabel("Messages in this conversation")
    }
}
