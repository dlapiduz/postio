import PostioFFI
import SwiftUI

/// The results' timeline (specs/010-focus-search T072; design §3.3,
/// screens 06 and 07): 66 tall -- the count line and its sub-line on the
/// left, a bar per month for the last twelve in the middle (36 tall at
/// most, 10.5 pt labels), and the hint on the right.
///
/// The heights are the controller's (`MonthBarFfi`), a share of the
/// tallest; a selected month -- inside the query's dates -- has the soft
/// band behind it and a bold label, and while any is selected the others
/// fade to the separator colour. A drag across the bars (step 4) is
/// reported once, on release, as the bars it covered (`TimelineDrag`);
/// the months that makes are the controller's. While a range is selected
/// the hint says so and names ⌥←/⌥→.
public struct TimelineView: View {
    let countLine: String
    let subLine: String
    let months: [MonthBarFfi]
    let hint: String
    let step: KeyHintFfi?
    @State private var drag: TimelineDrag

    public init(
        countLine: String, subLine: String, months: [MonthBarFfi], hint: String, step: KeyHintFfi? = nil,
        onMonths: @escaping (UInt32, UInt32) -> Void = { _, _ in }
    ) {
        self.countLine = countLine
        self.subLine = subLine
        self.months = months
        self.hint = hint
        self.step = step
        _drag = State(initialValue: TimelineDrag(bars: max(months.count, 12), report: onMonths))
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
            MonthChart(months: months, barHeight: Self.barHeight, labelSize: 10.5, drag: drag)
                .frame(maxWidth: .infinity)
            hintText
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

    /// The hint, and ⌥←/⌥→ "steps a month" after it while a range is
    /// selected (screen 09).
    private var hintText: Text {
        guard let step else { return Text(hint) }
        let cap = KeyCapSpelling.cap(step.key) ?? step.key
        return Text("\(hint) \u{b7} ") + Text(cap).font(.system(size: 11, design: .monospaced))
            + Text(" \(step.label)")
    }
}

/// Bars for months with their labels under them: the timeline's (36 tall)
/// and the Date popover's (90). A drag across them is `drag`'s to report.
public struct MonthChart: View {
    let months: [MonthBarFfi]
    let barHeight: CGFloat
    let labelSize: CGFloat
    let drag: TimelineDrag

    public init(months: [MonthBarFfi], barHeight: CGFloat, labelSize: CGFloat, drag: TimelineDrag) {
        self.months = months
        self.barHeight = barHeight
        self.labelSize = labelSize
        self.drag = drag
    }

    private var anySelected: Bool { months.contains(where: \.selected) || drag.band != nil }

    private func selected(_ at: Int, _ month: MonthBarFfi) -> Bool {
        if let band = drag.band { return band.contains(at) }
        return month.selected
    }

    public var body: some View {
        VStack(spacing: 3) {
            GeometryReader { proxy in
                HStack(alignment: .bottom, spacing: 4) {
                    ForEach(Array(months.enumerated()), id: \.offset) { at, month in
                        ZStack(alignment: .bottom) {
                            UnevenRoundedRectangle(topLeadingRadius: 4, topTrailingRadius: 4)
                                .fill(selected(at, month) ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear))
                            UnevenRoundedRectangle(topLeadingRadius: 2, topTrailingRadius: 2)
                                .fill(barFill(at, month))
                                .frame(height: max(barHeight * month.height, month.conversations > 0 ? 3 : 0))
                                .padding(.horizontal, 3)
                        }
                        .frame(maxWidth: .infinity)
                        .frame(height: barHeight)
                        .accessibilityElement()
                        .accessibilityLabel("\(month.label), \(month.conversations)")
                    }
                }
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in
                            drag.changed(
                                start: value.startLocation.x, at: value.location.x, width: proxy.size.width)
                        }
                        .onEnded { value in
                            drag.ended(
                                start: value.startLocation.x, at: value.location.x, width: proxy.size.width)
                        }
                )
            }
            .frame(height: barHeight)
            HStack(spacing: 2) {
                ForEach(Array(months.enumerated()), id: \.offset) { at, month in
                    Text(month.label)
                        .font(.system(size: labelSize, weight: selected(at, month) ? .bold : .regular))
                        // Twelve three-letter months in the Date popover's
                        // 270 points: tightened rather than cut to "M…".
                        .allowsTightening(true)
                        .minimumScaleFactor(0.75)
                        .foregroundStyle(
                            selected(at, month) ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
                        .frame(maxWidth: .infinity)
                        .accessibilityHidden(true)
                }
            }
            .lineLimit(1)
        }
    }

    private func barFill(_ at: Int, _ month: MonthBarFfi) -> AnyShapeStyle {
        if anySelected { return selected(at, month) ? AnyShapeStyle(.secondary) : AnyShapeStyle(.separator) }
        return AnyShapeStyle(.tertiary)
    }
}

/// A drag across a month chart's bars (specs/010-focus-search T083,
/// §3.3): reported once, when the pointer lets go, as the first and last
/// bar it covered in either direction -- a click is one bar -- so a drag
/// is one search, not one per bar it crosses. While it moves, `band` is the
/// bars it covers, for the chart to draw.
@MainActor
@Observable
public final class TimelineDrag {
    /// The bars it covers while the pointer is down.
    public private(set) var band: ClosedRange<Int>?
    @ObservationIgnored public let bars: Int
    @ObservationIgnored private let report: (UInt32, UInt32) -> Void

    public init(bars: Int, report: @escaping (UInt32, UInt32) -> Void) {
        self.bars = max(bars, 1)
        self.report = report
    }

    /// The bar under `x`, across a chart `width` wide; past either end,
    /// the end bar.
    public func bar(at x: Double, width: Double) -> Int {
        guard width > 0 else { return 0 }
        let at = Int((x / width * Double(bars)).rounded(.down))
        return max(0, min(bars - 1, at))
    }

    private func covered(start: Double, at x: Double, width: Double) -> ClosedRange<Int> {
        let (a, b) = (bar(at: start, width: width), bar(at: x, width: width))
        return min(a, b)...max(a, b)
    }

    /// The pointer moved, the drag having begun at `start`.
    public func changed(start: Double, at x: Double, width: Double) {
        band = covered(start: start, at: x, width: width)
    }

    /// The pointer let go at `x`: the bars covered are reported.
    public func ended(start: Double, at x: Double, width: Double) {
        let range = covered(start: start, at: x, width: width)
        band = nil
        report(UInt32(range.lowerBound), UInt32(range.upperBound))
    }
}
