import PostioFFI
import SwiftUI

/// The words of the footer under the list (spec 013 US2, FR-014; screen 02):
/// the scope in bold, keycap and label pairs, and the right-hand sentence.
/// The controller words all three after every cursor move, and keycaps come
/// from the live keymap; this holds none of its own.
public struct ScopeFooterWords: Equatable {
    public let title: String
    public let hints: [FocusHintRow.Hint]
    public let note: String?

    public init(_ view: FooterViewFfi) {
        title = view.title
        hints = view.keys.map { FocusHintRow.Hint(cap: $0.cap, label: $0.label) }
        note = view.note
    }
}

/// The footer, drawn: one line, the bold title, the hints, then the note
/// right-aligned in secondary text. When the line is too narrow the note
/// gives way first, then the hints; the title never does.
/// Semantic colours only (FR-017).
public struct ScopeFooter: View {
    let words: ScopeFooterWords

    public init(words: ScopeFooterWords) {
        self.words = words
    }

    public static var height: CGFloat { 34 }

    public var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 14) {
                Text(words.title)
                    .font(.system(size: 12, weight: .bold))
                    .lineLimit(1)
                    .layoutPriority(3)
                ForEach(words.hints) { hint in
                    HStack(spacing: 5) {
                        KeyCap(hint.cap)
                        Text(hint.label).font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(hint.label), key \(hint.cap)")
                    .layoutPriority(2)
                }
                Spacer(minLength: 12)
                if let note = words.note {
                    Text(note)
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(0)
                }
            }
            .lineLimit(1)
            .padding(.horizontal, 16)
            .frame(maxHeight: .infinity)
        }
        .frame(height: Self.height)
        .background(.bar)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(words.title)
    }
}
