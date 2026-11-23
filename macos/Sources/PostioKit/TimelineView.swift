import PostioFFI
import SwiftUI

/// Heights are the controller's (`MonthBarFfi`). A drag across the bars is reported once, on
/// release, as the bars it covered (`TimelineDrag`); the months are the controller's.
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

    /// Left to wrap at the column's width the hint broke wherever it ran out, leaving "to narrow"
    /// alone on the second line at 1440.
    static func hintLines(_ hint: String) -> String {
        guard let dot = hint.range(of: " \u{b7} ") else { return hint }
        return hint[..<dot.lowerBound] + "\n" + hint[dot.upperBound...]
    }
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

    private var hintText: Text {
        let hint = Self.hintLines(hint)
        guard let step else { return Text(hint) }
        let cap = KeyCapSpelling.cap(step.key) ?? step.key
        return Text("\(hint) \u{b7} ") + Text(cap).font(.system(size: 11, design: .monospaced))
            + Text(" \(step.label)")
    }
}

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

    /// "Jul, 9+" when the count is a floor: the controller's words, not the number.
    static func spoken(_ month: MonthBarFfi) -> String {
        "\(month.label), \(month.count)"
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
                        .accessibilityLabel(Self.spoken(month))
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
                        // Twelve three-letter months in 270 points: tightened rather than cut to
                        // "M…".
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

/// Reported once, on release, as the first and last bar covered in either direction (a click is one
/// bar), so a drag is one search, not one per bar. While it moves, `band` is what the chart draws.
@MainActor
@Observable
public final class TimelineDrag {
    public private(set) var band: ClosedRange<Int>?
    @ObservationIgnored public let bars: Int
    @ObservationIgnored private let report: (UInt32, UInt32) -> Void

    public init(bars: Int, report: @escaping (UInt32, UInt32) -> Void) {
        self.bars = max(bars, 1)
        self.report = report
    }

    public func bar(at x: Double, width: Double) -> Int {
        guard width > 0 else { return 0 }
        let at = Int((x / width * Double(bars)).rounded(.down))
        return max(0, min(bars - 1, at))
    }

    private func covered(start: Double, at x: Double, width: Double) -> ClosedRange<Int> {
        let (a, b) = (bar(at: start, width: width), bar(at: x, width: width))
        return min(a, b)...max(a, b)
    }

    public func changed(start: Double, at x: Double, width: Double) {
        band = covered(start: start, at: x, width: width)
    }

    public func ended(start: Double, at x: Double, width: Double) {
        let range = covered(start: start, at: x, width: width)
        band = nil
        report(UInt32(range.lowerBound), UInt32(range.upperBound))
    }
}
