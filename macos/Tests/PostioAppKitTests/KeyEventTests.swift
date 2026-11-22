import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

@Suite struct KeyEventTests {
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
        // `shift+a` is `A` to the resolver: the canvas binds `a` and `A` to different commands.
        let reduced = KeyEvent.reduce(keyDown("A", modifiers: .shift))
        #expect(reduced?.character == "A")
        #expect(reduced?.modifiers.shift == true)
    }

    @Test func commandIsTheModifierTheKeymapCallsSuper() {
        let reduced = KeyEvent.reduce(keyDown("k", modifiers: .command))
        #expect(reduced?.character == "k")
        #expect(reduced?.modifiers.command == true)
        #expect(reduced?.modifiers.control == false)
    }

    @Test func aNamedKeyCrossesAsItsNameAndNoCharacter() {
        // AppKit spells these as control characters and private-use scalars; the resolver's table
        // is GDK's, so the adapter translates (ADR 0019 Q4).
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
        // `from_platform_key` names Space itself; naming it here too would be a second decider.
        let reduced = KeyEvent.reduce(keyDown(" "))
        #expect(reduced?.character == " ")
        #expect(reduced?.name == nil)
    }

    @Test func aKeyThatProducedNothingIsNoEvent() {
        // A lone modifier and a dead key mid-composition must propagate, or non-Latin keyboards
        // break.
        #expect(KeyEvent.reduce(keyDown("")) == nil)
    }

    @Test func theDeviceIndependentFlagsAreWhatIsRead() {
        // The raw flags carry left/right and numeric-keypad bits; a chord matching only left
        // Command would be irreproducible.
        let held = KeyEvent.held([.command, .shift, .function, .numericPad])
        #expect(held.command == true)
        #expect(held.shift == true)
        #expect(held.control == false)
        #expect(held.option == false)
    }
}
