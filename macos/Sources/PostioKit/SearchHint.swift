/// What the search field says about itself when nobody is using it.
///
/// The field is on screen and reachable with the mouse, which is most of
/// #1260 — but a keyboard-first application that never mentions its own keys
/// teaches nobody anything. `/` was in `docs/keybindings.md` and nowhere a
/// person would see it.
///
/// **From the keymap, never written down.** `search` is rebindable like
/// everything else, and a placeholder promising `/` to somebody who moved it
/// is worse than one that promises nothing.
public enum SearchHint {
    /// The field's placeholder, naming the key that focuses it.
    ///
    /// The **bare** key, not the chord. `search` carries both `/` and
    /// `alt+cmd+f`; there is room for one, and the chord is already in the
    /// menu, which is where chords are looked for. The mnemonic layer is the
    /// one that has to be taught, because nothing else shows it.
    public static func placeholder(bindings: [String]) -> String {
        let base = "Search mail"
        // A sequence — `g s` — is two presses and reads as a typo beside a
        // field. Nothing is better than a key somebody cannot press.
        guard let key = bindings.first(where: { !$0.contains(" ") && $0.count == 1 })
            ?? bindings.first(where: { !$0.contains(" ") })
        else {
            return base
        }
        // Two spaces, not one: it is a label beside the prompt rather than
        // part of the sentence, and a single space reads as "Search mail /".
        return "\(base)  \(key)"
    }
}
