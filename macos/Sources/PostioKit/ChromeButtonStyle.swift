import SwiftUI

/// `.plain` draws no hover or pressed fill, which left the row reading as text. Fills change at
/// once, as reduced motion asks.
public struct ChromeButtonStyle: ButtonStyle {
    let radius: CGFloat

    public init(radius: CGFloat = PostioTokens.radiusSm) {
        self.radius = radius
    }

    public func makeBody(configuration: Configuration) -> some View {
        Body(label: configuration.label, pressed: configuration.isPressed, radius: radius)
    }

    /// A view because only a view can keep hover state and read `isEnabled`.
    private struct Body<Label: View>: View {
        let label: Label
        let pressed: Bool
        let radius: CGFloat
        @State private var hovered = false
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            label
                .background(fill, in: RoundedRectangle(cornerRadius: radius))
                .onHover { hovered = $0 }
                .opacity(enabled ? 1 : 0.4)
        }

        private var fill: Color {
            guard enabled else { return .clear }
            if pressed { return Color(nsColor: PostioTokens.colorSelectedBg) }
            if hovered { return Color(nsColor: PostioTokens.colorHoverBg) }
            return .clear
        }
    }
}
