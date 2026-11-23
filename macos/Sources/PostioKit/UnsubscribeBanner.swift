import PostioFFI
import SwiftUI

/// Only on deliberate activation: nothing happens when the banner appears, since reading the offer
/// writes nothing and the one call that records is behind the button. The boundary refuses to
/// record for a message that offers nothing. Per message: a conversation can hold messages from
/// several lists.
public struct UnsubscribeBanner: View {
    private let offer: UnsubscribeOfferFfi
    /// Held by the engine, not here: `X` and the palette press the same thing (#1706), and a
    /// command cannot reach `@State`.
    private let state: Unsubscribing.State
    /// The `unsubscribe` command, the same path the key takes.
    private let leave: () -> Void
    private let clear: (Bool?) -> Void
    private let clearCap: String?

    public init(
        offer: UnsubscribeOfferFfi, state: Unsubscribing.State, leave: @escaping () -> Void,
        clear: @escaping (Bool?) -> Void = { _ in }, clearCap: String? = nil
    ) {
        self.clearCap = clearCap
        self.offer = offer
        self.state = state
        self.leave = leave
        self.clear = clear
    }

    public var body: some View {
        // Gone once recorded: the banner disappearing is the success.
        if state != .left {
            VStack(alignment: .leading, spacing: PostioTokens.space1) {
                HStack(spacing: PostioTokens.space3) {
                    Image(systemName: "envelope.open")
                        .foregroundStyle(.secondary)
                    if case .clearing(let sweep) = state {
                        // Left with mail still here: the offer replaces the button. Nothing moves
                        // until it is pressed, it is one undo, and dismissing keeps everything
                        // (#1952).
                        Text(sweep.summary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer(minLength: PostioTokens.space2)
                        Button {
                            clear(sweep.delete)
                        } label: {
                            HStack(spacing: 6) {
                                Text(sweep.button)
                                if let clearCap { KeyCap(clearCap) }
                            }
                        }
                        .buttonStyle(ReaderButtonStyle(.secondary))
                        Button("Dismiss") { clear(nil) }
                            .buttonStyle(ReaderButtonStyle(.secondary))
                            .keyboardShortcut(.cancelAction)
                    } else {
                        Text(offer.summary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer(minLength: PostioTokens.space2)
                        Button(state == .leaving ? "Leaving…" : offer.action, action: leave)
                            .buttonStyle(ReaderButtonStyle(.secondary))
                            .disabled(state == .leaving)
                    }
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
