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
    private let message: Int64
    private let session: PostioSession

    /// Whether the activation is in flight. It writes, and a write queues
    /// behind whatever the sync engine is committing — on a first sync that
    /// is not a few milliseconds, so the banner says what it is doing.
    @State private var leaving = false
    /// What went wrong, if it did. Never a success message: the banner
    /// disappearing *is* the success, and a green tick under a row that has
    /// gone is a claim about nothing.
    @State private var failure: String?
    /// Whether this message's list has been left, so the banner goes.
    @State private var left = false

    public init(offer: UnsubscribeOfferFfi, message: Int64, session: PostioSession) {
        self.offer = offer
        self.message = message
        self.session = session
    }

    public var body: some View {
        if !left {
            VStack(alignment: .leading, spacing: PostioTokens.space1) {
                HStack(spacing: PostioTokens.space3) {
                    Image(systemName: "envelope.open")
                        .foregroundStyle(.secondary)
                    Text(offer.summary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: PostioTokens.space2)
                    Button(leaving ? "Leaving…" : offer.action) { leave() }
                        .controlSize(.small)
                        .disabled(leaving)
                }
                if let failure {
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

    /// Record the activation — the one deliberate act.
    ///
    /// Off the main actor, which the boundary asks for by name: this is the
    /// only call in the pair that writes, and a write waits on the store's
    /// machine-wide gate.
    private func leave() {
        failure = nil
        leaving = true
        let session = session
        let message = message
        Task {
            let complaint = await Task.detached { session.activateUnsubscribe(message) }.value
            leaving = false
            if let complaint {
                failure = complaint
            } else {
                left = true
            }
        }
    }
}
