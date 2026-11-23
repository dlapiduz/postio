import Foundation
import PostioFFI
import SwiftUI

public struct UndoPillWords: Equatable {
    public let text: String
    public let undo: String?
    public let cap: String?
    public let accessibility: String

    public init(toast: FocusIntents.Toast, undoCap: String?) {
        text = toast.text
        let undo = toast.offersUndo ? "Undo" : nil
        self.undo = undo
        cap = undo == nil ? nil : undoCap
        var said = toast.text
        if let undo {
            said += ". \(undo)"
            if let undoCap { said += ", \(undoCap)" }
        }
        accessibility = said
    }
}

/// Driven by the controller's `FocusToast`: a new toast replaces the showing one and restarts the
/// timer. ⌘Z keeps working after the pill goes: the pill is a reminder, the stack is the undo. Fade
/// follows `Motion.current` (≤ 100 ms, none under Reduce Motion).
public struct UndoPill: View {
    let toast: FocusIntents.Toast
    let token: Int
    let undoCap: String?
    let undo: () -> Void
    let dismiss: (Int) -> Void

    public init(
        toast: FocusIntents.Toast, token: Int, undoCap: String?,
        undo: @escaping () -> Void, dismiss: @escaping (Int) -> Void
    ) {
        self.toast = toast
        self.token = token
        self.undoCap = undoCap
        self.undo = undo
        self.dismiss = dismiss
    }

    public static let height: CGFloat = 46
    public static let margin: CGFloat = 20

    public var body: some View {
        let words = UndoPillWords(toast: toast, undoCap: undoCap)
        HStack(spacing: 24) {
            Text(words.text)
                .font(.system(size: 14))
                .foregroundStyle(.background)
                .lineLimit(1)
            Spacer(minLength: 0)
            if let label = words.undo {
                Button(action: undo) {
                    HStack(spacing: 7) {
                        Text(label).font(.system(size: 14, weight: .bold))
                        if let cap = words.cap { InvertedKeyCap(cap) }
                    }
                    .foregroundStyle(.background)
                    .padding(.horizontal, 12)
                    .frame(height: 32)
                    .background(Capsule().fill(.background.opacity(0.16)))
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(words.cap.map { "\(label), key \($0)" } ?? label)
            }
        }
        .padding(.leading, 18)
        .padding(.trailing, words.undo == nil ? 18 : 7)
        .frame(height: Self.height)
        .frame(minWidth: 320)
        .fixedSize()
        .background(Capsule().fill(.primary))
        .shadow(radius: 8, y: 2)
        .padding(.bottom, Self.margin)
        .transition(.opacity)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(words.accessibility)
        .accessibilityAddTraits(.updatesFrequently)
        .task(id: token) {
            try? await Task.sleep(nanoseconds: UInt64(toast.seconds * 1_000_000_000))
            guard !Task.isCancelled else { return }
            dismiss(token)
        }
    }
}

private struct InvertedKeyCap: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(.system(size: KeyCapMetrics.fontSize, design: .monospaced))
            .foregroundStyle(.background.opacity(0.85))
            .lineLimit(1)
            .padding(.horizontal, KeyCapMetrics.padding)
            .frame(minWidth: KeyCapMetrics.minWidth, minHeight: KeyCapMetrics.height,
                   maxHeight: KeyCapMetrics.height)
            .background(
                RoundedRectangle(cornerRadius: KeyCapMetrics.radius).fill(.background.opacity(0.14))
            )
            .overlay(
                RoundedRectangle(cornerRadius: KeyCapMetrics.radius)
                    .strokeBorder(.background.opacity(0.35), lineWidth: KeyCapMetrics.outline)
            )
            .accessibilityHidden(true)
    }
}
