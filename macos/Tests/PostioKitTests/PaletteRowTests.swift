import PostioFFI
import Testing

@testable import PostioKit

/// Turning match offsets into something AppKit can draw.
///
/// The ranking and the filtering are asserted in Rust, where they live
/// (`ffi_suite/palette.rs`). What is Swift's is this conversion, and its one
/// real hazard: the offsets are **bytes**, Swift strings are not indexed by
/// bytes, and a title with a non-ASCII character in it is where the naive
/// version emphasises the wrong glyph or traps.
@Suite struct PaletteRowTests {
    private func entry(
        title: String,
        positions: [UInt32],
        id: String = "archive"
    ) -> PaletteEntryFfi {
        PaletteEntryFfi(id: id, title: title, binding: nil, positions: positions)
    }

    @Test func theMatchedCharactersAreEmphasised() {
        let drawn = PaletteRow.highlighted(entry(title: "Archive", positions: [0, 1, 2]))
        let runs = drawn.runs.map { (String(drawn[$0.range].characters), $0.inlinePresentationIntent) }
        #expect(runs.first?.0 == "Arc")
        #expect(runs.first?.1 == .stronglyEmphasized)
    }

    @Test func anUnmatchedTitleIsDrawnPlain() {
        let drawn = PaletteRow.highlighted(entry(title: "Archive", positions: []))
        #expect(String(drawn.characters) == "Archive")
        #expect(drawn.runs.allSatisfy { $0.inlinePresentationIntent == nil })
    }

    @Test func aMultiByteTitleEmphasisesTheRightCharacter() {
        // "Move to…" — the ellipsis is three bytes, so a byte offset past it
        // is not the character offset. Getting this wrong emphasises the
        // wrong glyph, or splits one and produces mojibake.
        let title = "Move to… somewhere"
        let bytesBeforeS = title.prefix(while: { $0 != "s" }).utf8.count
        let drawn = PaletteRow.highlighted(entry(title: title, positions: [UInt32(bytesBeforeS)]))
        let emphasised = drawn.runs
            .filter { $0.inlinePresentationIntent == .stronglyEmphasized }
            .map { String(drawn[$0.range].characters) }
        #expect(emphasised == ["s"], "emphasised \(emphasised) instead of the `s`")
    }

    @Test func anOffsetPastTheEndCostsAnEmphasisAndNotACrash() {
        // The offsets arrive from another language across an FFI. A mismatch
        // should cost a missing bold in a palette, never a trap in one.
        let drawn = PaletteRow.highlighted(entry(title: "Archive", positions: [99]))
        #expect(String(drawn.characters) == "Archive")
    }
}
