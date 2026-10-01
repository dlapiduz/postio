import SwiftUI

/// A binding as keycaps: one per press, so `g g` is two caps and `⇧⌘N` one.
/// A command with no key says so with a dash, which is an answer -- "this
/// exists and has no key" -- rather than a gap.
public struct KeyCaps: View {
    let binding: String?

    public init(binding: String?) {
        self.binding = binding
    }

    public var body: some View {
        let caps = CheatSheetKeys.caps(binding)
        if caps.isEmpty {
            Text("—").foregroundStyle(.tertiary)
        } else {
            HStack(spacing: 3) {
                ForEach(Array(caps.enumerated()), id: \.offset) { _, cap in
                    Text(cap)
                        .font(.system(size: 11, weight: .medium, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 5)
                        .frame(minWidth: 20, minHeight: 18)
                        .background(
                            RoundedRectangle(cornerRadius: PostioTokens.radiusMd)
                                .fill(Color.primary.opacity(0.06))
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: PostioTokens.radiusMd)
                                .strokeBorder(Color.primary.opacity(0.16), lineWidth: 1)
                        )
                }
            }
        }
    }
}
