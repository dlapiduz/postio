import PostioFFI
import SwiftUI

// The composer's *What to forward* chooser (spec 013 US7, FR-023; the Mac
// threads handoff SPEC section 6, screen 10): three cards above To, each with
// its number key, the chosen one ringed in the accent.
//
// The controller composes the cards and the words (`postio_ui::compose`) and
// decides what a choice does: `1` `2` `3` (the registry's
// `forward_what_1..3`, bound in `UiContext.forwardChooser`) and a click both
// end in the controller asking for the draft and the engine rebuilding it.
// This draws, and says which card was clicked.

/// How a key reaches the chooser, and what it asks.
public enum ForwardChooserKeys {
    /// The registry's command for a card: what its number key runs.
    public static func command(for form: ForwardFormFfi) -> String {
        switch form {
        case .this: return "forward_what_1"
        case .thread: return "forward_what_2"
        case .attachments: return "forward_what_3"
        }
    }

    /// The card a chooser command stands for; `nil` for any other command.
    public static func form(for command: String) -> ForwardFormFfi? {
        switch command {
        case "forward_what_1": return .this
        case "forward_what_2": return .thread
        case "forward_what_3": return .attachments
        default: return nil
        }
    }

    /// The contexts a key in the compose window is asked in, first to last.
    ///
    /// The chooser holds the keyboard while it is shown and no text field
    /// does: a `2` typed into To or the body is a `2`. Its context answers
    /// first, and anything it does not bind (Escape, Send) falls through to
    /// the composer's, so a shown chooser takes nothing from the window.
    public static func contexts(base: UiContext, chooserShown: Bool, typing: Bool) -> [UiContext] {
        guard base == .composer, chooserShown, !typing else { return [base] }
        return [.forwardChooser, base]
    }
}

/// The three cards.
public struct ForwardChooserView: View {
    let chooser: ChooserFfi
    let choose: (ForwardFormFfi) -> Void

    /// `choose` is a click on a card.
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

/// One card: the title and its number key, the line under them, a ring
/// when chosen.
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

/// One forwarded message, folded: "1 · Ana Ruiz · 2026-09-21 09:12 · Kicking
/// off v3...".
public struct ForwardedRow: Equatable, Sendable {
    /// Its place in the thread, from 1.
    public let number: Int
    /// Who and when, as the forward headed it.
    public let header: String
    /// Its first words.
    public let first: String

    /// What the collapsed row says.
    public var words: String {
        first.isEmpty ? "\(number) \u{b7} \(header)" : "\(number) \u{b7} \(header) \u{b7} \(first)"
    }
}

/// A forwarded thread's body, told apart from what is typed above it.
///
/// The engine writes the content as `postio_body::forward` does: a line
/// `---------- Forwarded messages ----------`, then each message as a header
/// line (`Who · 2026-09-21 09:12`) and its words. The marker is also what the
/// engine finds the typed text by, so folding at it hides nothing that is
/// typed.
public enum ForwardedRows {
    /// The line that opens a forwarded thread.
    static let marker = "---------- Forwarded messages ----------"

    /// `body` with the forwarded thread folded away, or `nil` for a body
    /// that forwards none.
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

    /// The messages in `quote`, one row each.
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

    /// `Who · 2026-09-21 09:12`.
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

/// The forwarded messages as rows, under what is typed: each a quiet bar,
/// with the key that opens them all.
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
