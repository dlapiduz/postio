import PostioFFI
import Testing

@testable import PostioKit

@Suite struct KeyDispositionTests {
    @Test func aCommandThatRanIsSwallowed() {
        #expect(KeyDisposition.swallows(outcome: .command(id: "archive"), acted: true))
    }

    @Test func aCommandNothingActedOnIsLeftToTheViewUnderneath() {
        #expect(
            !KeyDisposition.swallows(outcome: .command(id: "scroll_reader_down"), acted: false)
        )
    }

    @Test func aHalfTypedSequenceIsAlwaysSwallowed() {
        // `g` must not also be typed into whatever takes text next.
        #expect(KeyDisposition.swallows(outcome: .pending(description: "g"), acted: false))
    }

    @Test func aKeyThatResolvedToNothingIsNeverTaken() {
        #expect(!KeyDisposition.swallows(outcome: .unhandled, acted: false))
        #expect(!KeyDisposition.swallows(outcome: .unhandled, acted: true))
    }

    private func chord(_ character: String, shift: Bool = false) -> KeyEvent.Reduced {
        KeyEvent.Reduced(
            character: character, name: nil,
            modifiers: ModifiersFfi(control: false, option: false, shift: shift, command: true)
        )
    }

    @Test func selectAllInATextFieldSelectsTheText() {
        // `⌘A` in the add-account path field once resolved to "select all messages" and the field
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
        #expect(!KeyDisposition.belongsToText(chord("a"), typing: false))
    }

    @Test func otherCommandChordsStillReachTheirCommandsWhileTyping() {
        #expect(!KeyDisposition.belongsToText(chord("k"), typing: true))
    }

    private func named(_ name: String, shift: Bool = false, command: Bool = false) -> KeyEvent.Reduced {
        KeyEvent.Reduced(
            character: nil, name: name,
            modifiers: ModifiersFfi(control: false, option: false, shift: shift, command: command))
    }

    @Test func theFilesGridKeepsItsBareArrows() {
        for arrow in ["left", "right", "up", "down"] {
            #expect(KeyDisposition.belongsToGrid(named(arrow), gridFocused: true), "\(arrow)")
            #expect(!KeyDisposition.belongsToGrid(named(arrow), gridFocused: false), "\(arrow), elsewhere")
        }
        #expect(!KeyDisposition.belongsToGrid(named("down", command: true), gridFocused: true))
        #expect(!KeyDisposition.belongsToGrid(named("left", shift: true), gridFocused: true))
        #expect(!KeyDisposition.belongsToGrid(named("return"), gridFocused: true))
        #expect(
            !KeyDisposition.belongsToGrid(
                KeyEvent.Reduced(
                    character: "j", name: nil,
                    modifiers: ModifiersFfi(control: false, option: false, shift: false, command: false)),
                gridFocused: true))
    }

    private func key(
        _ character: String?, name: String? = nil,
        command: Bool = false, option: Bool = false, control: Bool = false
    ) -> KeyEvent.Reduced {
        KeyEvent.Reduced(
            character: character, name: name,
            modifiers: ModifiersFfi(control: control, option: option, shift: false, command: command)
        )
    }

    @Test func aCommandChordIsNeverTheInputMethods() {
        // Marked text sits at the caret mid-composition and after inline predictive text; ⌘↩ in the
        // compose body went to the web view, which has no Send.
        #expect(!KeyDisposition.belongsToInputMethod(key(nil, name: "return", command: true), composing: true))
        #expect(!KeyDisposition.belongsToInputMethod(key("b", command: true), composing: true))
    }

    @Test func whatAnInputMethodComposesWithStaysWithIt() {
        #expect(KeyDisposition.belongsToInputMethod(key(nil, name: "return"), composing: true))
        #expect(KeyDisposition.belongsToInputMethod(key(nil, name: "escape"), composing: true))
        #expect(KeyDisposition.belongsToInputMethod(key("k"), composing: true))
        #expect(KeyDisposition.belongsToInputMethod(key("e", option: true), composing: true))
        #expect(KeyDisposition.belongsToInputMethod(key(" ", control: true), composing: true))
    }

    @Test func withNothingMarkedNoKeyIsTheInputMethods() {
        #expect(!KeyDisposition.belongsToInputMethod(key("k"), composing: false))
        #expect(!KeyDisposition.belongsToInputMethod(key(nil, name: "return"), composing: false))
    }
}
