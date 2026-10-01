/// What the keyboard sheet prints for a binding.
///
/// The menu's glyphs (`MenuPlan.accelerator(from:)`), applied chord by chord,
/// so a sequence -- `g g`, which no menu can show -- still prints as its key
/// presses in turn. The sheet printed the binding as `[keys]` spells it,
/// `cmd+k`, beside a menu bar that says ⌘K for the same command.
public enum CheatSheetKeys {
    /// One cap per press: a sequence is two presses, a chord is one however
    /// many keys are held for it. Empty when the command has no key.
    public static func caps(_ binding: String?) -> [String] {
        guard let binding else { return [] }
        return binding
            .split(separator: " ")
            .map { chord in MenuPlan.accelerator(from: String(chord)) ?? String(chord) }
    }

    /// The caps as one line, or a dash when the command has none -- what a
    /// screen reader or a plain-text surface gets.
    public static func label(_ binding: String?) -> String {
        let caps = caps(binding)
        return caps.isEmpty ? "—" : caps.joined(separator: " ")
    }
}
