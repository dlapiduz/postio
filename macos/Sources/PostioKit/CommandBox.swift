/// The search box's command mode: `>` in the box.
///
/// GTK's finder is one box with several modes, a prefix in an empty box
/// choosing which question it asks (`postio_ui::finder::MODES`); `>` is
/// "Run a command", and `Ctrl+K` opens the box with it typed. The Mac had a
/// separate palette window for the same thing -- a third surface where the
/// design is converging on one -- so ⌘K here now does what it does there.
///
/// Holds only what the box decides: whether it is asking for a command, the
/// name typed so far, and which row the keyboard is on. The rows and their
/// order are `session.paletteEntries`, the shared matcher's.
public struct CommandBox: Equatable, Sendable {
    /// The character that asks for a command, `postio_ui::finder`'s.
    public static let marker: Character = ">"
    /// What ⌘K puts in the box.
    public static let opening = String(marker)

    /// The row the keyboard is on.
    public private(set) var highlighted = 0

    public init() {}

    /// The command name typed in `field`, or `nil` when the box is a search.
    ///
    /// Only a box that *starts* with the marker: a search may contain the
    /// character (`price >10`), and that does not change what it asks.
    public static func query(in field: String) -> String? {
        guard field.first == marker else { return nil }
        return String(field.dropFirst()).trimmingCharacters(in: .whitespaces)
    }

    /// ↑ or ↓, kept in the list.
    public mutating func move(by delta: Int, among count: Int) {
        guard count > 0 else {
            highlighted = 0
            return
        }
        highlighted = min(max(highlighted + delta, 0), count - 1)
    }

    /// The name changed: the best match is first again.
    public mutating func queryChanged() {
        highlighted = 0
    }
}
