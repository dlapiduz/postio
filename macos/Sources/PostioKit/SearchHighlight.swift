import Foundation
import PostioFFI

/// Drawing a search excerpt with its matches marked.
///
/// *What* matched is decided in Rust and crosses as byte ranges — the same
/// arrangement `postio_ui::palette` uses since #568, and for the same reason:
/// a highlighter on each side is two answers to one question, and the one that
/// drifts is invisible until somebody notices the wrong word is bold.
///
/// What is left here is narrow and still worth testing. The ranges are **byte**
/// offsets, and Swift's `String` is indexed by character; a name with an
/// accent, or the em dash `postio-search` cuts excerpts with, shifts every
/// character offset while leaving the byte offsets exactly right.
public enum SearchHighlight {
    /// The attribute a marked run carries.
    ///
    /// `inlinePresentationIntent` rather than a colour: the excerpt has to read
    /// correctly in both appearances and under Increase Contrast, and a colour
    /// chosen here would be a fourth place the design tokens are decided.
    static let mark: InlinePresentationIntent = .stronglyEmphasized

    /// `snippet` as an `AttributedString`, with each matched range marked.
    ///
    /// A range that is out of bounds, or that does not land on a character
    /// boundary, is skipped rather than trapped on. `String.Index(utf8Offset:)`
    /// mid-scalar is undefined, and an excerpt drawn without its highlighting
    /// is a far better outcome than an application that quits while the user is
    /// still typing the query.
    public static func attributed(_ snippet: SnippetFfi) -> AttributedString {
        var attributed = AttributedString(snippet.text)
        let utf8 = snippet.text.utf8
        for range in snippet.ranges {
            guard range.start < range.end,
                  let lower = index(in: snippet.text, atByte: Int(range.start)),
                  let upper = index(in: snippet.text, atByte: Int(range.end)),
                  Int(range.end) <= utf8.count,
                  let marked = Range(lower..<upper, in: attributed)
            else { continue }
            attributed[marked].inlinePresentationIntent = mark
        }
        return attributed
    }

    /// The index `byte` bytes into `text`, or `nil` if that is not a character
    /// boundary or is past the end.
    private static func index(in text: String, atByte byte: Int) -> String.Index? {
        guard byte >= 0, byte <= text.utf8.count else { return nil }
        let utf8Index = text.utf8.index(text.utf8.startIndex, offsetBy: byte)
        // `samePosition(in:)` is exactly the boundary question, and answers
        // `nil` rather than trapping when the offset splits a scalar.
        return utf8Index.samePosition(in: text)
    }

    /// The substrings that came back marked.
    ///
    /// Test support, and deliberately reading the attributes back rather than
    /// re-deriving them from the ranges — an assertion that recomputed the
    /// answer would pass however wrong the marking was.
    static func markedSubstrings(of attributed: AttributedString) -> [String] {
        attributed.runs
            .filter { $0.inlinePresentationIntent == mark }
            .map { String(attributed[$0.range].characters) }
    }
}
