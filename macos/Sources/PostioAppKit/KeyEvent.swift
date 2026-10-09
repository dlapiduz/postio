import AppKit
import PostioFFI
import PostioKit

/// The AppKit half of `KeyEvent`: reducing an `NSEvent`. The value type it
/// reduces to lives in PostioKit, which must not import AppKit (#1264).
extension KeyEvent {
    /// Reduce a key-down event, or answer `nil` for one that means nothing.
    ///
    /// `charactersIgnoringModifiers` rather than `characters`, and the
    /// difference is the whole of what a European keyboard does: it applies
    /// Shift and ignores Option and Command, which is exactly the
    /// normalization `Chord::from_platform_key` expects — `shift+a` *is* `A`,
    /// while `alt+a` must stay `a` and not become `å`.
    ///
    /// `nil` for a modifier-only press and for a dead key mid-composition:
    /// both are keys the resolver has no answer for, and a monitor that
    /// swallowed them would break every non-Latin keyboard.
    public static func reduce(_ event: NSEvent) -> Reduced? {
        let modifiers = held(event.modifierFlags)
        guard let scalar = event.charactersIgnoringModifiers?.unicodeScalars.first else {
            // A modifier on its own, or a key that produced nothing at all.
            return nil
        }

        if let named = Self.name(for: scalar) {
            return Reduced(character: nil, name: named, modifiers: modifiers)
        }
        // A control character with no name of its own is not something a
        // binding can be written for; leave it to whatever AppKit does.
        guard !CharacterSet.controlCharacters.contains(scalar) else { return nil }
        return Reduced(character: String(scalar), name: nil, modifiers: modifiers)
    }

    /// The modifiers, as the boundary spells them.
    ///
    /// `.deviceIndependentFlagsMask` because the raw flags carry left/right
    /// distinctions and the numeric-keypad bit, and a chord that matched only
    /// the left Command key would be a bug nobody could reproduce.
    public static func held(_ flags: NSEvent.ModifierFlags) -> ModifiersFfi {
        let flags = flags.intersection(.deviceIndependentFlagsMask)
        return ModifiersFfi(
            control: flags.contains(.control),
            option: flags.contains(.option),
            shift: flags.contains(.shift),
            command: flags.contains(.command)
        )
    }

    /// The resolver's name for a key that types no useful character.
    ///
    /// The spellings are `postio_ui::keymap`'s alias table, which is GDK's —
    /// deliberately, so that one `[keys]` file and one `docs/keybindings.md`
    /// describe both platforms. Translating here rather than teaching the core
    /// about AppKit is the direction ADR 0019 Q4 chose: the core owns the
    /// vocabulary and each frontend owns the adapter into it.
    ///
    /// Returns `nil` for anything that is an ordinary character, including
    /// Space — `from_platform_key` turns a space into `Space` itself, and a
    /// second place deciding that is a second place to get it wrong.
    static func name(for scalar: Unicode.Scalar) -> String? {
        switch scalar.value {
        case 0x0D, 0x03: return "return"  // Return, and the keypad's Enter.
        case 0x09, 0x19: return "tab"  // Tab, and Shift-Tab's back-tab.
        case 0x1B: return "escape"
        case 0x7F: return "backspace"  // The key labelled Delete on a Mac.
        case UInt32(NSUpArrowFunctionKey): return "up"
        case UInt32(NSDownArrowFunctionKey): return "down"
        case UInt32(NSLeftArrowFunctionKey): return "left"
        case UInt32(NSRightArrowFunctionKey): return "right"
        case UInt32(NSDeleteFunctionKey): return "delete"  // Forward delete.
        case UInt32(NSInsertFunctionKey): return "insert"
        case UInt32(NSHomeFunctionKey): return "home"
        case UInt32(NSEndFunctionKey): return "end"
        case UInt32(NSPageUpFunctionKey): return "page_up"
        case UInt32(NSPageDownFunctionKey): return "page_down"
        case UInt32(NSMenuFunctionKey): return "menu"
        case UInt32(NSF1FunctionKey)...UInt32(NSF12FunctionKey):
            return "f\(scalar.value - UInt32(NSF1FunctionKey) + 1)"
        default: return nil
        }
    }
}
