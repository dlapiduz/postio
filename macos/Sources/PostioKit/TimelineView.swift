import PostioFFI
import SwiftUI

/// The results' timeline (specs/010-focus-search T072; design §3.3,
/// screens 06 and 07): 66 tall -- the count line and its sub-line on the
/// left, a bar per month for the last twelve in the middle (36 tall at
/// most, 10.5 pt labels), and the hint on the right.
///
/// Display only in step 3: dragging across months (`SetMonths`) and ⌥←/⌥→
/// arrive with step 4. The heights are the controller's (`MonthBarFfi`),
/// a share of the tallest; a selected month -- inside the query's dates --
/// has the soft band behind it and a bold label, and while any is
/// selected the others fade to the separator colour.
public struct TimelineView: View {
    let countLine: String
    let subLine: String
    let months: [MonthBarFfi]
    let hint: String

    public init(countLine: String, subLine: String, months: [MonthBarFfi], hint: String) {
        self.countLine = countLine
        self.subLine = subLine
        self.months = months
        self.hint = hint
    }

    public static let height: CGFloat = 66
    /// The tallest bar.
    public static let barHeight: CGFloat = 36

    public var body: some View {
        HStack(alignment: .bottom, spacing: 20) {
            VStack(alignment: .leading, spacing: 2) {
                Text(countLine).font(.system(size: 13, weight: .bold)).foregroundStyle(.primary)
                Text(subLine).font(.system(size: 12)).foregroundStyle(.tertiary)
            }
            .lineLimit(1)
            .padding(.bottom, 4)
            .frame(width: 210, alignment: .leading)
            chart.frame(maxWidth: .infinity)
            Text(hint)
                .font(.system(size: 12))
                .foregroundStyle(.tertiary)
                .multilineTextAlignment(.trailing)
                .lineSpacing(2)
                .padding(.bottom, 6)
                .frame(width: 230, alignment: .trailing)
        }
        .padding(.leading, 20)
        .padding(.trailing, 16)
        .padding(.bottom, 8)
        .frame(height: Self.height, alignment: .bottom)
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
    }

    private var anySelected: Bool { months.contains(where: \.selected) }

    private var chart: some View {
        VStack(spacing: 3) {
            HStack(alignment: .bottom, spacing: 4) {
                ForEach(Array(months.enumerated()), id: \.offset) { _, month in
                    ZStack(alignment: .bottom) {
                        UnevenRoundedRectangle(topLeadingRadius: 4, topTrailingRadius: 4)
                            .fill(month.selected ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear))
                        UnevenRoundedRectangle(topLeadingRadius: 2, topTrailingRadius: 2)
                            .fill(barFill(month))
                            .frame(height: max(Self.barHeight * month.height, month.conversations > 0 ? 3 : 0))
                            .padding(.horizontal, 3)
                    }
                    .frame(maxWidth: .infinity)
                    .frame(height: Self.barHeight)
                    .accessibilityElement()
                    .accessibilityLabel("\(month.label), \(month.conversations)")
                }
            }
            HStack(spacing: 4) {
                ForEach(Array(months.enumerated()), id: \.offset) { _, month in
                    Text(month.label)
                        .font(.system(size: 10.5, weight: month.selected ? .bold : .regular))
                        .foregroundStyle(month.selected ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                        .frame(maxWidth: .infinity)
                        .accessibilityHidden(true)
                }
            }
            .lineLimit(1)
        }
    }

    private func barFill(_ month: MonthBarFfi) -> AnyShapeStyle {
        if anySelected { return month.selected ? AnyShapeStyle(.secondary) : AnyShapeStyle(.separator) }
        return AnyShapeStyle(.tertiary)
    }
}
