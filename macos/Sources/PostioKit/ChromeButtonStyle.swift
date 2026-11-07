import SwiftUI

/// The open message's chrome buttons -- the action row, More's menu, and
/// the previous/next pair -- with a fill under the pointer and a stronger
/// one while pressed, so a person can see which button they are on and
/// that a click landed.
///
/// `.plain` draws neither, which left the row reading as text. The fills
/// are the list's own: `colorHoverBg` under the pointer, `colorSelectedBg`
/// while held. They change at once, with no animation, which is also what
/// reduced motion asks for.
public struct ChromeButtonStyle: ButtonStyle {
    let radius: CGFloat

    /// `radius` rounds the fill; `0` for a button inside a group that
    /// clips it, as the previous/next pair does.
    public init(radius: CGFloat = PostioTokens.radiusSm) {
        self.radius = radius
    }

    public func makeBody(configuration: Configuration) -> some View {
        Body(label: configuration.label, pressed: configuration.isPressed, radius: radius)
    }

    /// A view rather than inline, because only a view can keep hover state
    /// and read `isEnabled`.
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
