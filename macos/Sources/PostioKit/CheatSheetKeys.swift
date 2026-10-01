/// What the keyboard sheet prints for a binding.
///
/// The menu's glyphs (`MenuPlan.accelerator(from:)`), applied chord by chord,
/// so a sequence -- `g g`, which no menu can show -- still prints as its key
/// presses in turn. The sheet printed the binding as `[keys]` spells it,
/// `cmd+k`, beside a menu bar that says ⌘K for the same command.
public enum CheatSheetKeys {
    /// The label for `binding`, or a dash when the command has none.
    public static func label(_ binding: String?) -> String {
        guard let binding, !binding.isEmpty else { return "—" }
        return binding
            .split(separator: " ")
            .map { chord in MenuPlan.accelerator(from: String(chord)) ?? String(chord) }
            .joined(separator: " ")
    }
}
