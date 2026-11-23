import PostioFFI
import SwiftUI

public enum ForwardChooserKeys {
    public static func command(for form: ForwardFormFfi) -> String {
        switch form {
        case .this: return "forward_what_1"
        case .thread: return "forward_what_2"
        case .attachments: return "forward_what_3"
        }
    }

    public static func form(for command: String) -> ForwardFormFfi? {
        switch command {
        case "forward_what_1": return .this
        case "forward_what_2": return .thread
        case "forward_what_3": return .attachments
        default: return nil
        }
    }

    /// The chooser answers while it holds the keyboard and never while text is typed: a `2` typed
    /// into To is a `2`.
    /// Anything it does not bind (Escape, Send) falls through to the composer's.
    public static func contexts(base: UiContext, chooserFocused: Bool, typing: Bool) -> [UiContext] {
        guard base == .composer, chooserFocused, !typing else { return [base] }
        return [.forwardChooser]
    }
}

public struct ForwardChooserView: View {
    let chooser: ChooserFfi
    let choose: (ForwardFormFfi) -> Void

    public init(chooser: ChooserFfi, choose: @escaping (ForwardFormFfi) -> Void) {
        self.chooser = chooser
        self.choose = choose
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(chooser.heading)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
            HStack(alignment: .top, spacing: 10) {
                ForEach(chooser.cards, id: \.title) { card in
                    ForwardCard(card: card, selected: card.form == chooser.selected) {
                        choose(card.form)
                    }
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(chooser.heading)
    }
}

struct ForwardCard: View {
    let card: ChooserCardFfi
    let selected: Bool
    let action: () -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 8)
        Button(action: action) {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(card.title)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.primary)
                    Spacer(minLength: 4)
                    if let key = KeyCapSpelling.cap(card.key) { KeyCap(key) }
                }
                Text(card.detail)
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, minHeight: 64, alignment: .topLeading)
            .padding(10)
            .background(shape.fill(selected ? Color.accentColor.opacity(0.06) : .clear))
            .overlay(shape.strokeBorder(selected ? Color.accentColor : Color.secondary.opacity(0.35),
                                        lineWidth: selected ? 2 : 1))
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(card.title). \(card.detail)")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

public struct ForwardedRow: Equatable, Sendable {
    public let number: Int
    public let header: String
    public let first: String

    public var words: String {
        first.isEmpty ? "\(number) \u{b7} \(header)" : "\(number) \u{b7} \(header) \u{b7} \(first)"
    }
}

/// The marker is also what the engine finds the typed text by, so folding at it hides nothing
/// typed.
public enum ForwardedRows {
    static let marker = "---------- Forwarded messages ----------"

    public static func fold(_ body: String) -> QuoteFoldFfi? {
        var start = body.startIndex
        while start < body.endIndex {
            let lineEnd = body[start...].firstIndex(of: "\n") ?? body.endIndex
            if body[start..<lineEnd] == marker {
                let quote = String(body[start...])
                let count = rows(in: quote).count
                guard count > 0 else { return nil }
                return QuoteFoldFfi(
                    written: String(body[..<start]), quote: quote,
                    summary: count == 1 ? "1 forwarded message" : "\(count) forwarded messages")
            }
            start = lineEnd < body.endIndex ? body.index(after: lineEnd) : body.endIndex
        }
        return nil
    }

    public static func rows(in quote: String) -> [ForwardedRow] {
        var rows: [ForwardedRow] = []
        var waiting: String?
        var header: String?
        for line in quote.split(separator: "\n", omittingEmptySubsequences: false) {
            let text = String(line)
            if isHeader(text) {
                header = text
                waiting = ""
                rows.append(ForwardedRow(number: rows.count + 1, header: text, first: ""))
            } else if header != nil, waiting == "", !text.trimmingCharacters(in: .whitespaces).isEmpty {
                waiting = text
                let last = rows.removeLast()
                rows.append(ForwardedRow(number: last.number, header: last.header, first: text))
            }
        }
        return rows
    }

    static func isHeader(_ line: String) -> Bool {
        guard let at = line.range(of: " \u{b7} ", options: .backwards) else { return false }
        let when = Array(line[at.upperBound...])
        guard when.count == 16 else { return false }
        let shape: [Character] = Array("dddd-dd-dd dd:dd")
        for (character, wanted) in zip(when, shape) {
            if wanted == "d" ? !character.isASCII || !character.isNumber : character != wanted {
                return false
            }
        }
        return !line[..<at.lowerBound].isEmpty
    }
}

public struct ForwardedRowsView: View {
    let rows: [ForwardedRow]
    let cap: String?
    let expand: () -> Void

    public init(rows: [ForwardedRow], cap: String?, expand: @escaping () -> Void) {
        self.rows = rows
        self.cap = cap
        self.expand = expand
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(rows, id: \.number) { row in
                Button(action: expand) {
                    HStack(spacing: 8) {
                        Text(row.words)
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Spacer(minLength: 4)
                        if row.number == rows.count, let cap { KeyCap(cap) }
                    }
                    .padding(.horizontal, 10)
                    .frame(height: 28)
                    .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary.opacity(0.6)))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(row.words). Show the forwarded messages")
            }
        }
        .padding(.leading, 5)
        .padding(.bottom, 8)
    }
}
