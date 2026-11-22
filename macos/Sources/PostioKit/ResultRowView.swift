import PostioFFI
import SwiftUI

/// Matched words wear the find yellow with label-colour text, never the accent, which stays the
/// focus ring's. The ring, selection tint and hairline are `ResultsTable`'s row view's.
public struct ResultRowView: View {
    let row: ResultRowFfi?
    let selecting: Bool
    let onName: ((String) -> Void)?
    @Environment(\.colorScheme) private var scheme

    public init(row: ResultRowFfi?, selecting: Bool, onName: ((String) -> Void)? = nil) {
        self.row = row
        self.selecting = selecting
        self.onName = onName
    }

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
                // A date never ends in an ellipsis: one with its year takes the room it needs.
                Text(row.date)
                    .font(.system(size: 12, weight: row.unread ? .bold : .regular).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .fixedSize()
                    .frame(minWidth: Columns.date, alignment: .trailing)
            } else {
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
                .contentShape(Rectangle())
                .onTapGesture {
                    if let address = row.fromAddress { onName?(address) }
                }
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
                if let summary = row.summary {
                    Text(summary).font(.system(size: 13)).foregroundStyle(.secondary)
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
            if row.summary == nil {
                HStack(spacing: 8) {
                    if !row.sourceTag.isEmpty {
                        sourceChip(row.sourceTag, italic: row.sourceIsFile)
                    }
                    Text(SearchRuns.attributed(row.passage, size: 12.5, secondary: true, scheme: scheme))
                        .truncationMode(.tail)
                }
            }
        }
    }

    private func sourceChip(_ tag: String, italic: Bool) -> some View {
        Text(tag)
            .font(.system(size: 11))
            .italic(italic)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 6)
            .frame(height: 17)
            .background(RoundedRectangle(cornerRadius: 4).fill(.quaternary))
            .fixedSize()
    }

    private func labelPill(_ pill: LabelPillFfi) -> some View {
        HStack(spacing: 5) {
            Circle()
                .fill(LabelColour.of(name: pill.name, stored: pill.color).map { AnyShapeStyle($0.swatch) } ?? AnyShapeStyle(.tertiary))
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

public struct ResultChildView: View {
    let child: ResultChildFfi?
    @Environment(\.colorScheme) private var scheme

    public init(child: ResultChildFfi?) {
        self.child = child
    }

    public var body: some View {
        HStack(spacing: ResultRowView.Columns.gap) {
            Color.clear.frame(width: ResultRowView.Columns.gutter)
            switch child {
            case let .match(_, _, tag, passage, sender, initials, time):
                Text(initials)
                    .font(.system(size: 10, weight: .semibold))
                    .frame(width: CGFloat(FocusRowMetrics.childAvatar), height: CGFloat(FocusRowMetrics.childAvatar))
                    .background(Circle().fill(Color(nsColor: .tertiaryLabelColor)))
                Text(sender)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                    .fixedSize()
                Text(tag)
                    .font(.system(size: 11))
                    .italic(tag.hasSuffix("file"))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .frame(height: 17)
                    .background(RoundedRectangle(cornerRadius: 4).fill(.quaternary))
                    .fixedSize()
                Text(SearchRuns.attributed(passage, size: 12.5, secondary: true, scheme: scheme))
                    .truncationMode(.tail)
                Spacer(minLength: 0)
                Text(time)
                    .font(.system(size: 12.5).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .fixedSize()
            case let .gap(text):
                Text(text).font(.system(size: 12)).foregroundStyle(.tertiary)
                Spacer(minLength: 0)
            case nil:
                Spacer()
            }
        }
        .lineLimit(1)
        .padding(.leading, ResultRowView.Columns.leading)
        .padding(.trailing, ResultRowView.Columns.trailing)
        .frame(maxHeight: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Self.accessible(child))
    }

    static func accessible(_ child: ResultChildFfi?) -> String {
        switch child {
        case let .match(_, _, tag, passage, sender, _, time):
            "\(sender), \(tag), " + passage.map(\.text).joined() + ", \(time)"
        case let .gap(text): text
        case nil: ""
        }
    }
}

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

/// A matched run wears the find yellow (system yellow at 38% light, 28% dark) with label-colour
/// text.
public enum SearchRuns {
    public static func highlight(_ scheme: ColorScheme) -> Color {
        Color.yellow.opacity(scheme == .dark ? 0.28 : 0.38)
    }

    /// The design's `accsoft`: 8% in light, 14% in dark.
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
