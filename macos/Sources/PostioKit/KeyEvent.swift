import PostioFFI

/// The whole Swift side of the keyboard (ADR 0019 Q4): no keymap and no sequence logic, since
/// `postio_ui::keymap` owns that for both frontends. Kept apart from the monitor because this half
/// has decisions that need no running application to assert.
public enum KeyEvent {
    public struct Reduced: Equatable, Sendable {
        public let character: String?
        public let name: String?
        public let modifiers: ModifiersFfi

        public init(character: String?, name: String?, modifiers: ModifiersFfi) {
            self.character = character
            self.name = name
            self.modifiers = modifiers
        }
    }
}
