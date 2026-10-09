import Foundation
import PostioFFI
import SwiftUI

/// What the undo pill says (specs/009-focus-macos T093, screen 15): the
/// controller's words for what a verb did, and Undo with the key that does
/// the same while the stack can take it back.
public struct UndoPillWords: Equatable {
    /// "Archived 3 messages": the controller's, never composed here.
    public let text: String
    /// "Undo", while the toast is an undoable completion.
    public let undo: String?
    /// The keycap beside Undo: the binding in force for `undo` (⌘Z).
    public let cap: String?
    /// What VoiceOver reads for the pill as a whole.
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

/// The undo pill (specs/009-focus-macos T093; screen 15;
/// contracts/mac-surfaces.md "Undo pill"): a capsule at the bottom centre
/// of the main window saying what a verb did, with an Undo button and its
/// ⌘Z keycap while the engine's stack can take it back.
///
/// Driven by the controller's `FocusToast`: a new toast replaces the one
/// showing (`FocusIntents.toastToken` changes, so the timer starts again),
/// and it goes after the toast's seconds -- the controller's eight, or an
/// answer's own window. ⌘Z keeps working after it has gone: the pill is a
/// reminder, the stack is the undo (FR-041).
///
/// Drawn as the pack's default buttons are, filled with the primary label
/// colour and lettered in the background's (the Mac SPEC's "Colour"):
/// semantic, so light, dark and increased contrast all follow. Its fade is
/// the window's (`Motion.current`: ≤ 100 ms, none under Reduce Motion).
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
    /// Between the pill and the window's (or the action bar's) bottom.
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

/// A keycap on the pill's inverted fill: `KeyCap`'s geometry, its colours
/// turned over with the pill's.
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
