import SwiftUI

/// Square-shouldered, condensed heading voice: the primary verb filled with the accent, the rest
/// outlined. AppKit's bordered style truncated "Reply all" to "Reply…", so the label is the verb
/// alone and the chord is the tooltip.
public struct ReaderButtonStyle: ButtonStyle {
    public enum Kind: Sendable {
        case primary
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
