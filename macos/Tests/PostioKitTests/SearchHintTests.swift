import Testing

@testable import PostioKit

/// What the search field says about itself when nobody is using it.
///
/// #1260's last acceptance line: *"It says its own key, so the keyboard is
/// learned by using the app."* The field is on screen now and reachable with
/// the mouse — but a keyboard-first application that never mentions its own
/// keys teaches nobody anything, and `/` was documented in `docs/` and
/// nowhere a person would see it.
///
/// From the keymap rather than written down: `search` is rebindable like
/// everything else, and a placeholder promising `/` to somebody who moved it
/// is worse than a placeholder that promises nothing.
@Suite struct SearchHintTests {
    @Test func theFieldNamesTheKeyThatFocusesIt() {
        #expect(SearchHint.placeholder(bindings: ["/"]) == "Search mail  /")
    }

    @Test func aRebinderSeesTheirOwnKey() {
        // The whole reason it is not the literal `/`.
        #expect(SearchHint.placeholder(bindings: ["s"]) == "Search mail  s")
    }

    @Test func theMnemonicWinsOverTheChordBecauseItIsShorter() {
        // `search` carries `/` and `alt+cmd+f`. The placeholder has room for
        // one, and the bare key is the one worth teaching — the chord is
        // already in the menu, where chords live.
        #expect(SearchHint.placeholder(bindings: ["/", "alt+cmd+f"]) == "Search mail  /")
    }

    @Test func aCommandWithNoBindingPromisesNothing() {
        // Somebody who unbound it must not be told to press a key that does
        // nothing.
        #expect(SearchHint.placeholder(bindings: []) == "Search mail")
    }

    @Test func aSequenceIsNotOfferedAsAKeyToPress() {
        // `g g` is two presses and reads as a typo in a placeholder.
        #expect(SearchHint.placeholder(bindings: ["g s"]) == "Search mail")
    }
}
