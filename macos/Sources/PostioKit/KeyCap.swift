import SwiftUI

public enum KeyCapMetrics {
    public static let fontSize: CGFloat = 10
    public static let height: CGFloat = 15
    public static let radius: CGFloat = 3
    public static let outline: CGFloat = 1
    public static let padding: CGFloat = 4
    public static let minWidth: CGFloat = 15
}

public struct KeyCap: View {
    let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
        Text(text)
            .font(.system(size: KeyCapMetrics.fontSize, design: .monospaced))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .padding(.horizontal, KeyCapMetrics.padding)
            .frame(minWidth: KeyCapMetrics.minWidth, minHeight: KeyCapMetrics.height,
                   maxHeight: KeyCapMetrics.height)
            .background(
                RoundedRectangle(cornerRadius: KeyCapMetrics.radius).fill(.quaternary)
            )
            .overlay(
                RoundedRectangle(cornerRadius: KeyCapMetrics.radius)
                    .strokeBorder(.separator, lineWidth: KeyCapMetrics.outline)
            )
            .accessibilityHidden(true)
    }
}
