import PostioFFI
import Testing

@testable import PostioKit

/// Claiming a key and acting on it are two different things.
///
/// The key monitor runs ahead of the responder chain, so a key it swallows
/// never reaches AppKit. It swallowed everything the resolver claimed — and
/// a command this frontend has not built yet resolves just as well as one it
/// has. `space` in the reading pane resolved to `scroll_reader_down`, was
/// swallowed, reached no handler, and never got to the scroll view that
/// would have paged it natively.
///
/// Which is worse than a no-op. A key that does nothing reads as a broken
/// application; a key that does nothing *and* suppresses the platform's own
/// behaviour reads as one that is trying and failing.
@Suite struct KeyDispositionTests {
    @Test func aCommandThatRanIsSwallowed() {
        #expect(KeyDisposition.swallows(outcome: .command(id: "archive"), acted: true))
    }

    @Test func aCommandNothingActedOnIsLeftToTheViewUnderneath() {
        // The whole point: `space` that paged nothing has to reach the scroll
        // view, which pages.
        #expect(
            !KeyDisposition.swallows(outcome: .command(id: "scroll_reader_down"), acted: false)
        )
    }

    @Test func aHalfTypedSequenceIsAlwaysSwallowed() {
        // `g` must not also be typed into whatever takes text next, whether
        // or not anything has "acted" yet — nothing has, by definition.
        #expect(KeyDisposition.swallows(outcome: .pending(description: "g"), acted: false))
    }

    @Test func aKeyThatResolvedToNothingIsNeverTaken() {
        #expect(!KeyDisposition.swallows(outcome: .unhandled, acted: false))
        #expect(!KeyDisposition.swallows(outcome: .unhandled, acted: true))
    }

    // -- text editing belongs to the field (found driving the app) --------

    private func chord(_ character: String, shift: Bool = false) -> KeyEvent.Reduced {
        KeyEvent.Reduced(
            character: character, name: nil,
            modifiers: ModifiersFfi(control: false, option: false, shift: shift, command: true)
        )
    }

    @Test func selectAllInATextFieldSelectsTheText() {
        // `⌘A` in the add-account sheet's path field resolved to the
        // registry's "select all messages", was swallowed, and the field
        // never selected its text.
        #expect(KeyDisposition.belongsToText(chord("a"), typing: true))
    }

    @Test func everyEditingChordStaysWithTheField() {
        for key in ["a", "c", "v", "x", "z"] {
            #expect(KeyDisposition.belongsToText(chord(key), typing: true), "⌘\(key)")
        }
        #expect(KeyDisposition.belongsToText(chord("Z", shift: true), typing: true), "⇧⌘Z")
    }

    @Test func outsideAFieldTheChordsAreTheApplications() {
        // Over the list, `⌘A` *is* select all messages.
        #expect(!KeyDisposition.belongsToText(chord("a"), typing: false))
    }

    @Test func otherCommandChordsStillReachTheirCommandsWhileTyping() {
        // `⌘K` from the search field opens the palette; only editing is the
        // field's.
        #expect(!KeyDisposition.belongsToText(chord("k"), typing: true))
    }
}
