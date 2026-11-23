import PostioFFI

/// The core's sentence, with presentation decided here: how long it stays, whether it offers Undo,
/// which of two wins. Lives in `PostioKit` because the executable target cannot be tested.
public struct Notice: Equatable, Sendable {
    public let kind: NoticeKindFfi
    public let message: String
    public let undoable: Bool

    public init(kind: NoticeKindFfi, message: String, undoable: Bool) {
        self.kind = kind
        self.message = message
        self.undoable = undoable
    }

    public init?(_ event: UiEvent) {
        guard case let .notice(kind, message, undoable) = event else { return nil }
        self.init(kind: kind, message: message, undoable: undoable)
    }

    /// Only a completion: a refusal changed nothing and a failure did not finish, so there is
    /// nothing to undo.
    public var offersUndo: Bool { kind == .completed && undoable }

    /// Only a failure; a refusal is an ordinary keyboard slip and gets a quiet hint.
    public var isAlarming: Bool { kind == .failed }

    /// Only a failure: Focus's controller says every completion, undo and refusal as `FocusToast`,
    /// so drawing them here too would announce one archive twice.
    public var shownBesideFocusToast: Bool { kind == .failed }

    /// A completion that offers Undo outlasts the reach for the mouse; a refusal is shortest, a
    /// failure longest.
    public var seconds: Double {
        switch kind {
        case .refused: return 2
        case .undone: return 4
        case .completed: return offersUndo ? 6 : 4
        case .failed: return 8
        }
    }

    /// The registry's id, so the button and `u` are one command; `NoticeTests` holds it against the
    /// registry.
    public static let undoCommand = "undo"

    /// A failure outranks everything; otherwise the newer wins, except an undo replaces the
    /// completion it took back.
    public static func winner(showing: Notice?, arriving: Notice) -> Notice {
        guard let showing else { return arriving }
        if showing.isAlarming && !arriving.isAlarming { return showing }
        return arriving
    }
}
