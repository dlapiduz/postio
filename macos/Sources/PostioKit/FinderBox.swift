/// The search box's modes: a prefix chooses which question the box asks.
///
/// GTK's finder is one box with several modes (`postio_ui::finder::MODES`):
/// `>` runs a command, `#` goes to a folder, `@` finds a correspondent, `+`
/// labels the selection, and anything else searches. `Ctrl+K` opens the box
/// on `>`. The Mac had a separate palette window for the commands and none
/// of the other three, while its keyboard sheet -- read from the same table
/// -- listed all four.
///
/// Holds only what the box decides: which mode, the text after the prefix,
/// and which row the keyboard is on. The rows and their order are the
/// boundary's, scored by the shared matcher.
public struct FinderBox: Equatable, Sendable {
    /// Which question the box is asking.
    public enum Mode: Character, Equatable, Sendable, CaseIterable {
        case command = ">"
        case folder = "#"
        case contact = "@"
        case label = "+"
    }

    /// A mode and what has been typed after its prefix.
    public struct Asking: Equatable, Sendable {
        public let mode: Mode
        public let text: String

        public init(mode: Mode, text: String) {
            self.mode = mode
            self.text = text
        }
    }

    /// What ⌘K puts in the box.
    public static let commands = String(Mode.command.rawValue)

    /// The row the keyboard is on.
    public private(set) var highlighted = 0

    public init() {}

    /// The question `field` asks, or `nil` when it is a search.
    ///
    /// Only a box that *starts* with a prefix: a search may contain the
    /// character (`price >10`, `issue #12`), and that does not change what it
    /// asks.
    public static func asking(in field: String) -> Asking? {
        guard let first = field.first, let mode = Mode(rawValue: first) else { return nil }
        return Asking(mode: mode, text: String(field.dropFirst()).trimmingCharacters(in: .whitespaces))
    }

    /// ↑ or ↓, kept in the list.
    public mutating func move(by delta: Int, among count: Int) {
        guard count > 0 else {
            highlighted = 0
            return
        }
        highlighted = min(max(highlighted + delta, 0), count - 1)
    }

    /// What was typed changed: the best match is first again.
    public mutating func queryChanged() {
        highlighted = 0
    }
}

/// One row the box offers, whichever mode it is in.
public struct FinderRow: Equatable, Sendable, Identifiable {
    /// A command id, or a folder's, label's or correspondent's key.
    public let id: String
    public let title: String
    /// The second thing the row says: unread count, address.
    public let detail: String?
    /// Byte offsets in `title` the query matched.
    public let positions: [UInt32]
    /// A command's key, drawn as caps.
    public let binding: String?

    public init(id: String, title: String, detail: String?, positions: [UInt32], binding: String?) {
        self.id = id
        self.title = title
        self.detail = detail
        self.positions = positions
        self.binding = binding
    }
}
