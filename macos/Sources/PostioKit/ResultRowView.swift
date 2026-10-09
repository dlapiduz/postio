import PostioFFI
import SwiftUI

/// One result row (specs/010-focus-search T072; design §3.4, screens 06
/// and 07): the 28 gutter (the unread dot, or the box while selecting),
/// the 176 sender column with a top hit's reason under it, the content --
/// the subject with its label pills, paperclip and thread count, then the
/// source tag and the matching passage -- the 96 folder column in SF Mono
/// and the 62 date.
///
/// Matched words wear the find yellow with the text in label colour, never
/// the accent, which stays the focus ring's (FR-005). The ring, the
/// selection tint and the hairline are the table's row view's
/// (`ResultsTable`), so this draws only words.
public struct ResultRowView: View {
    let row: ResultRowFfi?
    /// Rows are being checked: the gutter shows a box, not the dot.
    let selecting: Bool
    @Environment(\.colorScheme) private var scheme

    public init(row: ResultRowFfi?, selecting: Bool) {
        self.row = row
        self.selecting = selecting
    }

    /// The columns of §3.4, and the gap between them.
    public enum Columns {
        public static let gutter: CGFloat = 28
        public static let sender: CGFloat = 176
        public static let folder: CGFloat = 96
        public static let date: CGFloat = 62
        public static let gap: CGFloat = 12
        public static let leading: CGFloat = 12
        public static let trailing: CGFloat = 20
    }

    public var body: some View {
        HStack(spacing: Columns.gap) {
            gutter.frame(width: Columns.gutter)
            if let row {
                sender(row).frame(width: Columns.sender, alignment: .leading)
                content(row).frame(maxWidth: .infinity, alignment: .leading)
                Text(row.folder)
                    .font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .frame(width: Columns.folder, alignment: .trailing)
                // A date never ends in an ellipsis: one with its year
                // ("22 Oct 25") takes the room it needs if the column is short.
                Text(row.date)
                    .font(.system(size: 12, weight: row.unread ? .bold : .regular).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .fixedSize()
                    .frame(minWidth: Columns.date, alignment: .trailing)
            } else {
                // Its page is on its way: the row's place, kept quiet.
                Spacer()
            }
        }
        .lineLimit(1)
        .padding(.leading, Columns.leading)
        .padding(.trailing, Columns.trailing)
        .frame(maxHeight: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row?.accessible ?? "")
        .accessibilityAddTraits(row?.checked == true ? .isSelected : [])
    }

    @ViewBuilder
    private var gutter: some View {
        if selecting {
            let checked = row?.checked == true
            RoundedRectangle(cornerRadius: 4)
                .fill(checked ? AnyShapeStyle(.tint) : AnyShapeStyle(.clear))
                .overlay(
                    RoundedRectangle(cornerRadius: 4)
                        .strokeBorder(checked ? AnyShapeStyle(.clear) : AnyShapeStyle(.tertiary), lineWidth: 1))
                .overlay {
                    if checked {
                        Image(systemName: "checkmark").font(.system(size: 9, weight: .bold)).foregroundStyle(.white)
                    }
                }
                .frame(width: 14, height: 14)
        } else if row?.unread == true {
            Circle().fill(.tint).frame(width: 7, height: 7)
        } else {
            Color.clear.frame(width: 7, height: 7)
        }
    }

    private func sender(_ row: ResultRowFfi) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(row.sender)
                .font(.system(size: 13.5, weight: row.unread ? .bold : .regular))
                .foregroundStyle(.primary)
                .truncationMode(.tail)
            if let reason = row.reason {
                Text(reason).font(.system(size: 11.5)).foregroundStyle(.tertiary).truncationMode(.tail)
            }
        }
    }

    private func content(_ row: ResultRowFfi) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Text(SearchRuns.attributed(
                    row.subject, size: row.topHit ? 14.5 : 14,
                    weight: row.unread || row.topHit ? .semibold : .regular, scheme: scheme))
                    .truncationMode(.tail)
                    .layoutPriority(1)
                ForEach(Array(row.pills.enumerated()), id: \.offset) { _, pill in
                    labelPill(pill)
                }
                if row.attachments {
                    Image(systemName: "paperclip").font(.system(size: 11)).foregroundStyle(.tertiary)
                        .accessibilityHidden(true)
                }
                if let badge = row.countBadge {
                    Text(badge)
                        .font(.system(size: 10.5).monospacedDigit())
                        .foregroundStyle(.tertiary)
                        .padding(.horizontal, 4)
                        .frame(minWidth: 18, minHeight: 16)
                        .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(.separator, lineWidth: 1))
                        .fixedSize()
                }
            }
            HStack(spacing: 8) {
                if !row.sourceTag.isEmpty {
                    Text(row.sourceTag)
                        .font(.system(size: 11))
                        .italic(row.sourceIsFile)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .frame(height: 17)
                        .background(RoundedRectangle(cornerRadius: 4).fill(.quaternary))
                        .fixedSize()
                }
                Text(SearchRuns.attributed(row.passage, size: 12.5, secondary: true, scheme: scheme))
                    .truncationMode(.tail)
            }
        }
    }

    private func labelPill(_ pill: LabelPillFfi) -> some View {
        HStack(spacing: 5) {
            Circle()
                .fill(LabelColour(hex: pill.color).map { AnyShapeStyle($0.swatch) } ?? AnyShapeStyle(.tertiary))
                .frame(width: 6, height: 6)
            Text(pill.name).font(.system(size: 11.5)).foregroundStyle(.secondary)
        }
        .padding(.leading, 6)
        .padding(.trailing, 7)
        .frame(height: 18)
        .overlay(Capsule().strokeBorder(.separator, lineWidth: 1))
        .fixedSize()
    }
}

/// A group's header (design §3.4): 32 tall, the title bold 12 secondary,
/// then the count and the note tertiary.
public struct ResultGroupHeaderView: View {
    let group: ResultGroupFfi?

    public init(group: ResultGroupFfi?) {
        self.group = group
    }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(group?.title ?? "").font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
            let tail = [group?.count, group?.note].compactMap { $0 }.filter { !$0.isEmpty }
            if !tail.isEmpty {
                Text(tail.joined(separator: " \u{b7} ")).font(.system(size: 12)).foregroundStyle(.tertiary)
            }
            Spacer(minLength: 0)
        }
        .lineLimit(1)
        .padding(.horizontal, 20)
        .padding(.top, 12)
        .frame(maxHeight: .infinity, alignment: .top)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(group?.accessible ?? "")
        .accessibilityAddTraits(.isHeader)
    }
}

/// Runs of words with the engine's highlights, as one attributed string
/// (FR-005): a matched run wears the find yellow -- the design's
/// `rgb(255,204,0)` at 38% in light, `rgb(255,214,10)` at 28% in dark,
/// which is the system yellow -- with its text in label colour.
public enum SearchRuns {
    public static func highlight(_ scheme: ColorScheme) -> Color {
        Color.yellow.opacity(scheme == .dark ? 0.28 : 0.38)
    }

    /// How much of the accent tints a focused row, card or way out: the
    /// design's `accsoft`, 8% in light and 14% in dark (screens 06, 07).
    public static func focusFill(_ scheme: ColorScheme) -> Double {
        scheme == .dark ? 0.14 : 0.08
    }

    public static func attributed(
        _ runs: [RunFfi], size: CGFloat, weight: Font.Weight = .regular, secondary: Bool = false,
        scheme: ColorScheme
    ) -> AttributedString {
        var text = AttributedString()
        for run in runs {
            var piece = AttributedString(run.text)
            switch run.style {
            case .strong:
                piece.font = .system(size: size, weight: .semibold)
                piece.foregroundColor = .primary
            case .mono:
                piece.font = .system(size: size - 0.5, design: .monospaced)
                piece.foregroundColor = .primary
            case .plain:
                piece.font = .system(size: size, weight: weight)
                piece.foregroundColor = secondary ? .secondary : .primary
            }
            if run.highlighted {
                piece.backgroundColor = highlight(scheme)
                piece.foregroundColor = .primary
            }
            text += piece
        }
        return text
    }
}
