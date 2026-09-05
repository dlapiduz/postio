import AppKit
import PostioFFI
import Testing

@testable import PostioKit

/// Reducing an `NSEvent` to what the resolver asks for.
///
/// The point of these is that the *adapter* is asserted, not the resolver:
/// `postio-ui` has its own suite for what a chord means, and a frontend whose
/// adapter produced the wrong three things would leave that suite perfectly
/// green while nothing worked. #656's acceptance criterion says the chord a
/// synthetic `NSEvent` produces is checked against `postio_ui`'s own table,
/// and this is the Swift half of that; `ffi_suite/keys.rs` is the other.
@Suite struct KeyEventTests {
    /// A key-down event, built the way AppKit would deliver one.
    ///
    /// `charactersIgnoringModifiers` is what the reduction reads, so it is
    /// what these set. It is the one that applies Shift and ignores Option,
    /// which is exactly the normalization `Chord::from_platform_key` expects.
    private func keyDown(
        _ characters: String,
        modifiers: NSEvent.ModifierFlags = []
    ) -> NSEvent {
        NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: modifiers,
            timestamp: 0,
            windowNumber: 0,
            context: nil,
            characters: characters,
            charactersIgnoringModifiers: characters,
            isARepeat: false,
            keyCode: 0
        )!
    }

    @Test func anOrdinaryLetterCrossesAsItsCharacter() {
        let reduced = KeyEvent.reduce(keyDown("a"))
        #expect(reduced?.character == "a")
        #expect(reduced?.name == nil)
        #expect(reduced?.modifiers.command == false)
    }

    @Test func shiftIsFoldedIntoTheCharacterRatherThanReportedBeside() {
        // `shift+a` *is* `A` to the resolver, because that is what a keyboard
        // delivers and because the canvas binds `a` and `A` to different
        // commands. AppKit agrees: `charactersIgnoringModifiers` applies Shift.
        let reduced = KeyEvent.reduce(keyDown("A", modifiers: .shift))
        #expect(reduced?.character == "A")
        #expect(reduced?.modifiers.shift == true)
    }

    @Test func commandIsTheModifierTheKeymapCallsSuper() {
        // ⌘ is what `mod` expands to on this platform, and the boundary
        // carries it as `command`. The mapping to `Modifiers::SUPER` is the
        // Rust side's; this asserts the flag arrives set.
        let reduced = KeyEvent.reduce(keyDown("k", modifiers: .command))
        #expect(reduced?.character == "k")
        #expect(reduced?.modifiers.command == true)
        #expect(reduced?.modifiers.control == false)
    }

    @Test func aNamedKeyCrossesAsItsNameAndNoCharacter() {
        // AppKit spells these as control characters and private-use scalars;
        // the resolver's table is GDK's. Translating here is the direction ADR
        // 0019 Q4 chose -- the core owns the vocabulary, each frontend owns
        // the adapter into it -- so these names are `postio_ui::keymap`'s.
        for (characters, expected) in [
            ("\r", "return"),
            ("\u{1b}", "escape"),
            ("\t", "tab"),
            ("\u{7f}", "backspace"),
            (String(UnicodeScalar(UInt32(NSUpArrowFunctionKey))!), "up"),
            (String(UnicodeScalar(UInt32(NSDownArrowFunctionKey))!), "down"),
            (String(UnicodeScalar(UInt32(NSPageUpFunctionKey))!), "page_up"),
            (String(UnicodeScalar(UInt32(NSF5FunctionKey))!), "f5"),
        ] {
            let reduced = KeyEvent.reduce(keyDown(characters))
            #expect(reduced?.name == expected, "\(expected) did not survive the reduction")
            #expect(reduced?.character == nil, "\(expected) also sent a character")
        }
    }

    @Test func spaceStaysACharacterForTheCoreToName() {
        // `from_platform_key` turns a space into the named key `Space` itself.
        // Naming it here as well would be a second place deciding it, and the
        // two would eventually differ.
        let reduced = KeyEvent.reduce(keyDown(" "))
        #expect(reduced?.character == " ")
        #expect(reduced?.name == nil)
    }

    @Test func aKeyThatProducedNothingIsNoEvent() {
        // A modifier pressed on its own, and a dead key mid-composition.
        // Both must propagate: a monitor that swallowed a composition would
        // break every non-Latin keyboard.
        #expect(KeyEvent.reduce(keyDown("")) == nil)
    }

    @Test func theDeviceIndependentFlagsAreWhatIsRead() {
        // The raw flags carry left/right distinctions and the numeric-keypad
        // bit. A chord that matched only the left Command key would be a bug
        // nobody could reproduce, because nobody knows which one they press.
        let held = KeyEvent.held([.command, .shift, .function, .numericPad])
        #expect(held.command == true)
        #expect(held.shift == true)
        #expect(held.control == false)
        #expect(held.option == false)
    }
}
