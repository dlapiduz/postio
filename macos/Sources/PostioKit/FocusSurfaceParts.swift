import SwiftUI

// Pieces the surfaces of slice 12 share (specs/009-focus-macos T113-T116):
// the default button the pack draws for Archive all, Create and Add task,
// a verb with its keycap, and a row of footer hints. Semantic colours only
// (FR-017): the default button is the label colour, filled, with the text
// turned over, so the accent stays reserved for markers and focus.

/// A default button (Archive all, Create, Add task): bold words and their
/// keycap on a `labelColor` fill, the text in the background's colour.
public struct FocusDefaultButton: View {
    let title: String
    let cap: String?
    let action: () -> Void

    public init(_ title: String, cap: String?, action: @escaping () -> Void) {
        self.title = title
        self.cap = cap
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: 7) {
                Text(title).font(.system(size: 13.5, weight: .bold))
                if let cap { FilledKeyCap(cap) }
            }
            .foregroundStyle(.background)
            .padding(.horizontal, 12)
            .frame(height: 30)
            .background(RoundedRectangle(cornerRadius: 7).fill(.primary))
            .contentShape(RoundedRectangle(cornerRadius: 7))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(cap.map { "\(title), key \($0)" } ?? title)
    }
}

/// A plain verb: its words, then its keycap (the action rows, Cancel,
/// "Edit rule and cadence").
public struct FocusVerbButton: View {
    let title: String
    let cap: String?
    let bold: Bool
    let action: () -> Void

    public init(_ title: String, cap: String?, bold: Bool = true, action: @escaping () -> Void) {
        self.title = title
        self.cap = cap
        self.bold = bold
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Text(title).font(.system(size: 13.5, weight: bold ? .semibold : .regular))
                if let cap { KeyCap(cap) }
            }
            .foregroundStyle(.primary)
            .padding(.horizontal, 8)
            .frame(height: 28)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(cap.map { "\(title), key \($0)" } ?? title)
    }
}

/// A keycap on a default button's fill: `KeyCap`'s geometry, its colours
/// turned over with the button's.
public struct FilledKeyCap: View {
    let text: String

    public init(_ text: String) {
        self.text = text
    }

    public var body: some View {
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

/// A footer of hints: each key as a cap, then its words, in a row.
public struct FocusHintRow: View {
    public struct Hint: Equatable, Identifiable {
        public let cap: String
        public let label: String
        public var id: String { cap + label }

        public init(cap: String, label: String) {
            self.cap = cap
            self.label = label
        }
    }

    let hints: [Hint]

    public init(_ hints: [Hint]) {
        self.hints = hints
    }

    public var body: some View {
        HStack(spacing: 14) {
            ForEach(hints) { hint in
                HStack(spacing: 5) {
                    KeyCap(hint.cap)
                    Text(hint.label).font(.system(size: 12)).foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel("\(hint.label), key \(hint.cap)")
            }
            Spacer(minLength: 0)
        }
        .lineLimit(1)
    }
}

/// The chip a reason or a reference sits in: the quaternary fill, the
/// secondary label colour, monospaced when it is a number.
public struct FocusChip: View {
    let text: String
    let mono: Bool

    public init(_ text: String, mono: Bool = false) {
        self.text = text
        self.mono = mono
    }

    public var body: some View {
        Text(text)
            .font(.system(size: mono ? 10.5 : 12, design: mono ? .monospaced : .default))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .padding(.horizontal, mono ? 4 : 7)
            .frame(minHeight: mono ? 15 : 20)
            .background(RoundedRectangle(cornerRadius: mono ? 3 : 4).fill(.quaternary))
    }
}
