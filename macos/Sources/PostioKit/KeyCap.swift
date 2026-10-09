import SwiftUI

/// The keycap's geometry, the one place it is written down (FR-019,
/// contracts/mac-surfaces.md "Keycap"): monospaced 10 pt, 15 pt tall, a
/// 1 px outline, radius 3. The SwiftUI `KeyCap` and AppKit's `KeyCapView`
/// both read it, so a cap in the header strip and a cap on a row's
/// action are the same element drawn twice, not two that drift.
public enum KeyCapMetrics {
    public static let fontSize: CGFloat = 10
    public static let height: CGFloat = 15
    public static let radius: CGFloat = 3
    public static let outline: CGFloat = 1
    /// The room either side of the text.
    public static let padding: CGFloat = 4
    /// A cap is never narrower than it is tall: `e` is a square, not a
    /// sliver.
    public static let minWidth: CGFloat = 15
}

/// One keycap: the text the keymap spells (`KeyCapSpelling`), in the
/// monospaced face, on the quaternary fill with a separator outline.
///
/// Semantic colours only, so light, dark and a changed accent all follow
/// without a restart (FR-017).
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
