import AppKit
import PostioFFI
import PostioKit

extension KeyEvent {
    /// `charactersIgnoringModifiers`, not `characters`: it applies Shift and ignores Option and
    /// Command, as `Chord::from_platform_key` expects (`shift+a` is `A`, `alt+a` stays `a`, not
    /// `å`). `nil` for a modifier-only press and a dead key mid-composition: swallowing them would
    /// break non-Latin keyboards.
    public static func reduce(_ event: NSEvent) -> Reduced? {
        let modifiers = held(event.modifierFlags)
        guard let scalar = event.charactersIgnoringModifiers?.unicodeScalars.first else {
            return nil
        }

        if let named = Self.name(for: scalar) {
            return Reduced(character: nil, name: named, modifiers: modifiers)
        }
        guard !CharacterSet.controlCharacters.contains(scalar) else { return nil }
        return Reduced(character: String(scalar), name: nil, modifiers: modifiers)
    }

    /// `.deviceIndependentFlagsMask`: the raw flags carry left/right and keypad bits, and a chord
    /// matching only the left Command key would be unreproducible.
    public static func held(_ flags: NSEvent.ModifierFlags) -> ModifiersFfi {
        let flags = flags.intersection(.deviceIndependentFlagsMask)
        return ModifiersFfi(
            control: flags.contains(.control),
            option: flags.contains(.option),
            shift: flags.contains(.shift),
            command: flags.contains(.command)
        )
    }

    /// The spellings are `postio_ui::keymap`'s alias table (GDK's), so one `[keys]` file describes
    /// both platforms (ADR 0019 Q4). `nil` for ordinary characters including Space, which
    /// `from_platform_key` handles itself.
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
