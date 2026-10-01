import PostioFFI
import SwiftUI

/// One-click unsubscribe, which `PRODUCT.md` lists among the privacy
/// features and which had no surface here at all (#1585).
///
/// **Only on deliberate activation.** That is the rule the whole feature is
/// built around, and it is why nothing happens when the banner *appears*:
/// reading the offer is a point read that writes nothing, and the single call
/// that records anything is behind this button. The boundary refuses to
/// record an activation for a message that offers nothing, so a frontend
/// cannot unsubscribe anybody from a message the reader never offered it on.
///
/// The sentence and the verb are both the boundary's — `offer.summary` and
/// `offer.action` — so the two frontends say the same thing about what is
/// being left.
///
/// Per message, like the blocked-images notice beside it: a conversation can
/// hold eight messages from four lists, and one banner above them all could
/// not say whose.
public struct UnsubscribeBanner: View {
    private let offer: UnsubscribeOfferFfi
    /// Where this message's offer has got to. Held by the engine, not here:
    /// `X` and the palette press the same thing this button does (#1706),
    /// and a command cannot reach a view's `@State`.
    private let state: Unsubscribing.State
    /// Run the `unsubscribe` command for this message -- the one deliberate
    /// act, and the same path the key takes.
    private let leave: () -> Void

    public init(offer: UnsubscribeOfferFfi, state: Unsubscribing.State, leave: @escaping () -> Void) {
        self.offer = offer
        self.state = state
        self.leave = leave
    }

    public var body: some View {
        // Gone once recorded: the banner disappearing *is* the success, and
        // a green tick under a row that has gone is a claim about nothing.
        if state != .left {
            VStack(alignment: .leading, spacing: PostioTokens.space1) {
                HStack(spacing: PostioTokens.space3) {
                    Image(systemName: "envelope.open")
                        .foregroundStyle(.secondary)
                    Text(offer.summary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: PostioTokens.space2)
                    Button(state == .leaving ? "Leaving…" : offer.action, action: leave)
                        .controlSize(.small)
                        .disabled(state == .leaving)
                }
                if case .failed(let failure) = state {
                    Text(failure)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.horizontal, PostioTokens.space3)
            .padding(.vertical, PostioTokens.space2)
            .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: 6))
            .accessibilityElement(children: .contain)
            .accessibilityLabel(offer.summary)
        }
    }
}
