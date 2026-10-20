/// Where each message's unsubscribe offer has got to, for this view.
///
/// A type rather than three `@State`s in `UnsubscribeBanner`, for the reason
/// `RenderedOnce` records: `unsubscribe` is a command (`X`, the palette), a
/// command cannot reach view state, and while the banner held its own the
/// button worked and the key could not (#1706). The banner's button runs the
/// same command now, so the two cannot drift.
///
/// This holds no consent and decides none. Whether there is a list to leave
/// is the boundary's (`messageFacts`), and what an activation records is
/// re-derived there from the message id alone (`activateUnsubscribe`); this
/// only remembers what happened on screen.
public struct Unsubscribing: Equatable, Sendable {
    /// What the banner for one message is showing.
    public enum State: Equatable, Sendable {
        /// The offer, untouched.
        case offered
        /// The activation is in flight: it writes, and a write can queue
        /// behind a sync's commit, so the banner says what it is doing.
        case leaving
        /// Recorded, and the list left mail behind: the banner stays to offer
        /// Archive or Delete for it (#1931). Neither is done until pressed.
        case clearing(Sweep)
        /// Recorded. The banner goes; its absence is the confirmation.
        case left
        /// Refused, with the boundary's sentence. Pressing again retries.
        case failed(String)
    }

    /// What the list left behind, in the boundary's words.
    public struct Sweep: Equatable, Sendable {
        public let summary: String
        public let archive: String
        public let delete: String

        public init(summary: String, archive: String, delete: String) {
            self.summary = summary
            self.archive = archive
            self.delete = delete
        }
    }

    private var states: [Int64: State] = [:]

    public init() {}

    public func state(of message: Int64) -> State { states[message] ?? .offered }

    /// Start leaving `message`'s list, or answer `false` when that would be
    /// a second activation -- one already in flight, or a list already left.
    public mutating func begin(_ message: Int64) -> Bool {
        switch state(of: message) {
        case .leaving, .left, .clearing:
            return false
        case .offered, .failed:
            states[message] = .leaving
            return true
        }
    }

    /// What the boundary answered: `nil` when the activation was recorded.
    public mutating func finish(_ message: Int64, complaint: String?) {
        states[message] = complaint.map(State.failed) ?? .left
    }

    /// The list is left and it left mail behind: offer to clear it.
    public mutating func offerSweep(_ message: Int64, _ sweep: Sweep) {
        guard state(of: message) == .left else { return }
        states[message] = .clearing(sweep)
    }

    /// The offer was answered, either way: the banner goes.
    public mutating func answerSweep(_ message: Int64) {
        if case .clearing = state(of: message) { states[message] = .left }
    }

    /// Take back a `begin` that turned out to have nothing to act on -- a
    /// key pressed over a message with no list, which leaves no trace.
    public mutating func withdraw(_ message: Int64) {
        states[message] = nil
    }

    /// Forget everything -- the pane is showing something else now.
    public mutating func clear() { states = [:] }
}
