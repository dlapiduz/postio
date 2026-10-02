import Testing

@testable import PostioKit

/// What the keyboard sheet prints for a binding: the menu's glyphs, chord by
/// chord, so a sequence can be printed too.
@Suite struct CheatSheetKeysTests {
    @Test func aChordIsDrawnTheWayTheMenuDrawsIt() {
        // It printed `cmd+k`: the binding as the config file spells it,
        // beside a menu bar that says ⌘K for the same command.
        #expect(CheatSheetKeys.label("cmd+k") == "⌘K")
        #expect(CheatSheetKeys.label("cmd+comma") == "⌘,")
        #expect(CheatSheetKeys.label("cmd+shift+n") == "⇧⌘N")
        #expect(CheatSheetKeys.label("Escape") == "⎋")
    }

    @Test func aFoldedCapitalShowsItsShift() {
        // `J` is `shift+j`; drawn bare it reads as the same key as `j`.
        #expect(CheatSheetKeys.label("J") == "⇧J")
        #expect(CheatSheetKeys.label("x") == "X")
    }

    @Test func aSequenceIsEachOfItsChordsInTurn() {
        // The one thing a menu cannot print, and the reason this sheet
        // exists: `g g` has no accelerator, but it has two key presses.
        #expect(CheatSheetKeys.label("g g") == "G G")
        #expect(CheatSheetKeys.label("g i") == "G I")
    }

    @Test func noBindingIsADash() {
        #expect(CheatSheetKeys.label(nil) == "—")
    }

    @Test func eachPressIsItsOwnCap() {
        // A sequence is two presses, so two caps; a chord is one press
        // however many keys are held, so one cap.
        #expect(CheatSheetKeys.caps("g g") == ["G", "G"])
        #expect(CheatSheetKeys.caps("cmd+shift+n") == ["⇧⌘N"])
        #expect(CheatSheetKeys.caps(nil).isEmpty)
    }
}
