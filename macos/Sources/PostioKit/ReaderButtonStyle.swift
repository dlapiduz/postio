import SwiftUI

/// The reading pane's buttons, as canvas 25 draws them.
///
/// Square-shouldered rectangles in the condensed heading voice: the primary
/// verb (Reply) filled with the accent, the rest outlined. AppKit's bordered
/// style is a rounded capsule that truncated "Reply all" to "Reply…" as soon
/// as a chord shared its label, which is the other half of this: the label is
/// the verb alone and is never truncated. The chord is the button's tooltip,
/// still discoverable, no longer competing for the width.
///
/// The system face at its condensed width stands in for Barlow Condensed,
/// which only the reader's web view has registered so far.
public struct ReaderButtonStyle: ButtonStyle {
    public enum Kind: Sendable {
        /// Filled with the accent: the one verb a person most likely wants.
        case primary
        /// Outlined: everything beside it.
        case secondary
    }

    let kind: Kind

    public init(_ kind: Kind) {
        self.kind = kind
    }

    public func makeBody(configuration: Configuration) -> some View {
        Body(kind: kind, label: configuration.label, pressed: configuration.isPressed)
    }

    /// A view rather than inline, because only a view can read `isEnabled`.
    private struct Body<Label: View>: View {
        let kind: Kind
        let label: Label
        let pressed: Bool
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            let shape = RoundedRectangle(cornerRadius: PostioTokens.radiusSm)
            label
                .font(.system(size: 13, weight: .semibold).width(.condensed))
                .lineLimit(1)
                .fixedSize()
                .padding(.horizontal, PostioTokens.space4)
                .padding(.vertical, PostioTokens.space1 + 1)
                .foregroundStyle(kind == .primary ? Color.white : Color.primary)
                .background(kind == .primary ? Color(nsColor: PostioTokens.colorAccent) : Color.clear, in: shape)
                .overlay(shape.strokeBorder(kind == .primary ? Color.clear : Color.secondary.opacity(0.5), lineWidth: 1))
                .contentShape(shape)
                .opacity(enabled ? (pressed ? 0.75 : 1) : 0.4)
        }
    }
}
