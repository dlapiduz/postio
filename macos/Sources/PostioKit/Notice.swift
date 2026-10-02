import PostioFFI

/// What Postio says back when you ask it to do something.
///
/// # The bug this is
///
/// Four core events are the *outcome* of an action — `ActionCompleted`,
/// `UndoPerformed`, `CommandRejected`, `Error` — and every one of them fell
/// to `UiEvent::Other` at the boundary and was dropped on this side. So a
/// send that failed said nothing, `a` with nothing selected said nothing, and
/// an archive that could be taken back never offered to be. The application
/// did the work and never answered, which is the worst thing a keyboard-first
/// application can do: the whole contract of pressing a letter is that
/// something tells you what it did.
///
/// # What is decided here, and why here
///
/// The *sentence* is the core's — already phrased for a person by the layer
/// that knows what happened, so a frontend that composed its own would be
/// writing the same sentence twice and getting it different. What is left for
/// this side is presentation: how long it stays, whether it offers Undo, and
/// which of two notices in flight wins.
///
/// It is a type in `PostioKit` rather than a few lines in `Engine` because
/// `Engine` is in the executable target and nothing can test it. Every
/// decision that ends up there is a decision with no test.
public struct Notice: Equatable, Sendable {
    /// Which of the four it is.
    public let kind: NoticeKindFfi
    /// What to say, verbatim from the core.
    public let message: String
    /// Whether the undo stack can take the action back.
    public let undoable: Bool

    public init(kind: NoticeKindFfi, message: String, undoable: Bool) {
        self.kind = kind
        self.message = message
        self.undoable = undoable
    }

    /// The notice `event` carries, or `nil` if it is not one.
    public init?(_ event: UiEvent) {
        guard case let .notice(kind, message, undoable) = event else { return nil }
        self.init(kind: kind, message: message, undoable: undoable)
    }

    /// Whether to draw an Undo affordance.
    ///
    /// Only a completion, whatever the boundary said. A refusal changed
    /// nothing and a failure did not finish, so there is nothing to return
    /// to — and an Undo button that undoes nothing is worse than no button.
    public var offersUndo: Bool { kind == .completed && undoable }

    /// Whether this is the kind that has to be hard to miss.
    ///
    /// Only a failure. A refusal is the user asking for something that does
    /// not apply, which is an ordinary thing to do with a keyboard and gets
    /// a quiet hint rather than an alarm — `postio_core::Event`'s own doc
    /// says so: *"Not an error: the UI usually answers with a quiet hint."*
    public var isAlarming: Bool { kind == .failed }

    /// How long it stays on screen.
    ///
    /// A completion that offers Undo has to outlast the reach for the mouse:
    /// `PRODUCT.md`'s undo window is the promise, and a toast that goes
    /// before anybody can press it is a promise not kept. A refusal is
    /// shortest — it is a hint about a key that did not apply — and a
    /// failure stays longest because it is the one somebody has to act on.
    public var seconds: Double {
        switch kind {
        case .refused: return 2
        case .undone: return 4
        case .completed: return offersUndo ? 6 : 4
        case .failed: return 8
        }
    }

    /// The command the Undo affordance runs.
    ///
    /// The registry's id, so the button and `u` are one command rather than
    /// two paths to one intention — a button with an undo of its own would be
    /// a second undo stack, and the only thing worse than no undo is two that
    /// disagree. `NoticeTests` holds this string against the registry,
    /// because a literal that stops naming a command is a button that
    /// silently does nothing.
    public static let undoCommand = "undo"

    /// Which of two notices is on screen when both are in flight.
    ///
    /// Two at once is ordinary: an action completes while a sync fails. A
    /// failure outranks everything, because it is the only one somebody has
    /// to do something about. Otherwise the newer one wins — except that an
    /// undo *replaces* the completion it took back, which is the same rule
    /// said from the other side: *Archived 12* then `u` then *Archived 12,
    /// undone* is one story, not two notices.
    public static func winner(showing: Notice?, arriving: Notice) -> Notice {
        guard let showing else { return arriving }
        if showing.isAlarming && !arriving.isAlarming { return showing }
        return arriving
    }
}
