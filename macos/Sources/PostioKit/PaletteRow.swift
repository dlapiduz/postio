import Foundation
import PostioFFI

/// Drawing one palette or cheat-sheet row.
///
/// **There is no matcher here, and there must not be.** `postio_ui::palette`
/// ranks the rows and `Session.paletteEntries` hands them over already
/// ordered; a second fuzzy match in Swift would mean the same query offers
/// different things on each platform, which is the drift ADR 0019 exists to
/// prevent. What is Swift's is turning the match *offsets* into something
/// AppKit can draw — the same numbers `postio-gtk` turns into Pango bold.
public enum PaletteRow {
    /// The title with the matched characters emphasised.
    ///
    /// The offsets that cross are **byte** offsets into the title, because
    /// that is what the matcher works in. Swift strings are not indexed by
    /// byte, so the conversion happens here, once, at the edge — and it is
    /// done by walking `utf8` rather than by assuming one byte per character,
    /// which is right until the first title with an em dash in it.
    public static func highlighted(_ entry: PaletteEntryFfi) -> AttributedString {
        var attributed = AttributedString(entry.title)
        guard !entry.positions.isEmpty else { return attributed }

        let wanted = Set(entry.positions)
        var offset = 0
        for character in entry.title {
            let width = String(character).utf8.count
            if wanted.contains(UInt32(offset)),
                let range = range(of: offset, width: width, in: attributed, title: entry.title)
            {
                attributed[range].inlinePresentationIntent = .stronglyEmphasized
            }
            offset += width
        }
        return attributed
    }

    /// The attributed range covering the character at byte `offset`.
    private static func range(
        of offset: Int,
        width: Int,
        in attributed: AttributedString,
        title: String
    ) -> Range<AttributedString.Index>? {
        guard let start = String.Index(utf8Offset: offset, in: title),
            let end = String.Index(utf8Offset: offset + width, in: title)
        else { return nil }
        return attributed.range(of: title[start..<end])
    }
}

private extension String.Index {
    /// The index `offset` bytes into `text`, or `nil` if that is not a
    /// character boundary.
    ///
    /// A boundary check rather than a trap: the offsets come from another
    /// language across an FFI, and a mismatch should cost a missing emphasis
    /// rather than a crash in a palette.
    init?(utf8Offset offset: Int, in text: String) {
        guard let index = text.utf8.index(
            text.utf8.startIndex, offsetBy: offset, limitedBy: text.utf8.endIndex
        ) else { return nil }
        guard let converted = index.samePosition(in: text) else { return nil }
        self = converted
    }
}
