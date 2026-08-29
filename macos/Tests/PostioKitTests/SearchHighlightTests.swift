import Testing
import PostioFFI
@testable import PostioKit

/// Turning the boundary's match ranges into something AppKit can draw.
///
/// The ranges are decided in Rust so both frontends agree about what matched
/// (#568, #660). What Swift owns is narrow and still easy to get wrong: byte
/// offsets are not `String.Index`, and a subject line with an em dash or an
/// accented name in front of the match shifts every character offset while
/// leaving the byte offsets correct.
@Suite("Search highlighting")
struct SearchHighlightTests {
    private func snippet(_ text: String, _ ranges: [(Int, Int)]) -> SnippetFfi {
        SnippetFfi(
            text: text,
            ranges: ranges.map { MatchRangeFfi(start: UInt32($0.0), end: UInt32($0.1)) }
        )
    }

    @Test("the matched span is the one the ranges point at")
    func marksTheMatch() {
        let marked = SearchHighlight.attributed(snippet("the quarterly numbers", [(4, 13)]))
        let runs = SearchHighlight.markedSubstrings(of: marked)
        #expect(runs == ["quarterly"])
    }

    @Test("every match is marked, not only the first")
    func marksEveryMatch() {
        let marked = SearchHighlight.attributed(
            snippet("quarterly and quarterly", [(0, 9), (14, 23)])
        )
        #expect(SearchHighlight.markedSubstrings(of: marked) == ["quarterly", "quarterly"])
    }

    @Test("byte offsets survive multi-byte text before the match")
    func handlesMultiByteText() {
        // "café — " is 10 bytes and 7 characters. A `String.Index` built by
        // counting characters would mark "uarterl" here, which is the kind of
        // bug that only ever shows up in somebody else's language.
        let text = "café — quarterly"
        let start = text.utf8.count - "quarterly".utf8.count
        let marked = SearchHighlight.attributed(
            snippet(text, [(start, text.utf8.count)])
        )
        #expect(SearchHighlight.markedSubstrings(of: marked) == ["quarterly"])
    }

    @Test("a range that does not land on a character boundary marks nothing")
    func refusesASplitCharacter() {
        // Rather than crash. `String.Index(utf8Offset:)` mid-scalar is a trap,
        // and an excerpt drawn without highlighting is a far better outcome
        // than an application that quits while you type.
        let marked = SearchHighlight.attributed(snippet("café", [(3, 4)]))
        #expect(SearchHighlight.markedSubstrings(of: marked).isEmpty)
        #expect(String(marked.characters) == "café", "the text still draws")
    }

    @Test("an out-of-bounds range is ignored rather than trapped on")
    func refusesAnOutOfBoundsRange() {
        let marked = SearchHighlight.attributed(snippet("short", [(2, 900)]))
        #expect(SearchHighlight.markedSubstrings(of: marked).isEmpty)
        #expect(String(marked.characters) == "short")
    }

    @Test("no ranges is plain text, not an error")
    func plainTextWhenNothingMatched() {
        // What a structured-only query answers: `is:unread` has nothing to
        // point at, and the excerpt is still worth drawing.
        let marked = SearchHighlight.attributed(snippet("nothing to point at", []))
        #expect(String(marked.characters) == "nothing to point at")
        #expect(SearchHighlight.markedSubstrings(of: marked).isEmpty)
    }
}
