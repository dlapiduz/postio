import PostioFFI

/// Reducing an `NSEvent` to the three things the resolver asks for.
///
/// **This is the whole of the Swift side of the keyboard** (ADR 0019 Q4).
/// There is no keymap here, no table of what `a` does, no notion of a
/// sequence: `postio_ui::keymap` owns all of that for both frontends, and
/// this hands it the character the key would type, the key's name when it
/// types none, and the modifiers held. The classic app's `Chord::from_key_event`
/// was the same twenty lines over GDK.
///
/// Kept apart from the monitor that installs it because this is the half with
/// decisions in it, and a decision that needs a running application and a real
/// keyboard to observe is a decision nothing asserts.
public enum KeyEvent {
    /// What `NSEvent` says, in the resolver's terms.
    public struct Reduced: Equatable, Sendable {
        /// The character the key would type, shift applied.
        public let character: String?
        /// The key's name, for a key that types nothing useful.
        public let name: String?
        /// What was held down with it.
        public let modifiers: ModifiersFfi

        public init(character: String?, name: String?, modifiers: ModifiersFfi) {
            self.character = character
            self.name = name
            self.modifiers = modifiers
        }
    }
}
